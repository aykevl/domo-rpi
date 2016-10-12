
use std::{env, fs, process, thread, time};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Sender, Receiver};

extern crate chrono;
extern crate crc8;
extern crate env_logger;
extern crate serde_json;
extern crate spidev;
extern crate ws;

mod peripheral;
mod messages;
mod socket;

use peripheral::*;
use messages::*;
use chrono::*;


const CMD_TEMP_NOW: u8 = 0x11;
const CMD_TEMP_AVG: u8 = 0x12;
const CMD_TEMP_RAW: u8 = 0x13;
const CMD_COLOR: u8 = 0x05;
const CMD_TEST: u8 = 0x20;

const LOG_INTERVAL: i64 = 60 * 5; // 5 minutes
const SERVER_URL: &'static str = "wss://domo.aykevl.nl/api/ws/device";
const CONFIG_PATH: &'static str = ".config/domo.json";
const SPIDEV_PATH: &'static str = "/dev/spidev0.0";
const COLOR_READ_TIMEOUT: u64 = 5; // 5 seconds


fn raw_to_celsius(value: u32, bits: u32) -> f64 {
    // Source: https://learn.adafruit.com/thermistor/using-a-thermistor
    // TODO: these constants should be read from the microcontroller
    let b_coefficient: f64 = 3950.0;  // β-coefficient
    let t0: f64 = 298.15;  // nominal temperature (25°C)
    let r0: f64 = 10000.0; // 10kΩ at 25°C
    let series_resistor: f64 = 10000.0; // 10kΩ series resistor

    // convert value to range 0..1, where 0.5 means t=t0
    let fvalue: f64 = value as f64 / (1 << bits) as f64;

    // calculate resistance for NTC
    let r = series_resistor / (1.0 / fvalue - 1.0);

    // Steinhart-Hart equation
    let tinv = (1.0 / t0) + 1.0 / b_coefficient * (r / r0).ln();
    let t = 1.0 / tinv;

    // convert from K to °C and return
    t - 273.15
}

fn decode_temp(value: u32) -> f64 {
    // Value holds temperature in centidegrees, where 0 equals -55°C.
    // Convert this value to regular °C readings.
    ((value as i32 - 5500) as f64) / 100.0
}

struct Domo {
    peripheral: Peripheral,
    color: Color,
}

fn log(domo: Arc<Mutex<Domo>>, tx_msg_to_server: Option<Arc<Mutex<Sender<String>>>>) {
    let now = Local::now();
    let temp = match domo.lock().unwrap().peripheral.read_number(CMD_TEMP_AVG, 2) {
        Ok(result) => Some(decode_temp(result)),
        Err(err) => {
            println!("failed to read temperature: {}", err);
            None
        }
    };
    match temp {
        Some(temp) => println!("{:02}:{:02} {:.2}°C", now.hour(), now.minute(), temp),
        None => {
            println!("{:02}:{:02} <none>", now.hour(), now.minute());
            return;
        }
    };

    // Send temperature when tx_sensor is not None.
    match tx_msg_to_server {
        Some(tx_msg_to_server) => {
            let msg = serde_json::to_string(&MsgSensorLog {
                    message: "sensorLog".to_string(),
                    name: "temp".to_string(),
                    value: temp.unwrap(),
                    time: now.timestamp(),
                    sensor_type: "temperature".to_string(),
                    interval: LOG_INTERVAL,
                })
                .unwrap();
            tx_msg_to_server.lock().unwrap().send(msg).unwrap();
        }
        None => {}
    }
}

fn actuator_to_server(domo: Arc<Mutex<Domo>>, tx_msg_to_server: Arc<Mutex<Sender<String>>>) {
    loop {
        thread::sleep(time::Duration::from_secs(COLOR_READ_TIMEOUT));

        let mut domo = domo.lock().unwrap();

        let color_raw = match domo.peripheral.read_number(CMD_COLOR, 4) {
            Ok(val) => val,
            Err(err) => {
                println!("could not read color: {}", err);
                continue;
            }
        };

        if domo.color.raw() == color_raw {
            continue;
        }
        domo.color.update(color_raw);

        println!("color change from peripheral: {:?}", domo.color);
        let msg = serde_json::to_string(&MsgColor {
                message: "actuator".to_string(),
                name: "color".to_string(),
                value: domo.color.clone(),
            })
            .unwrap();
        tx_msg_to_server.lock().unwrap().send(msg).unwrap();
    }
}

fn msg_from_server(domo: Arc<Mutex<Domo>>, rx_msg_from_server: Receiver<MsgServer>) {
    loop {
        let msg = rx_msg_from_server.recv().unwrap();
        if msg.message == "actuator" {
            if msg.name.is_none() {
                println!("WARNING: no name sent with actuator message: {:?}", &msg);
                continue;
            }
            let name = msg.name.unwrap();

            if msg.value.is_none() {
                println!("WARNING: no value sent in message");
                continue;
            }
            let value = msg.value.unwrap();

            match &name[..] {
                "color" => {
                    println!("color change from server: {:?}", value);
                    let mut domo = domo.lock().unwrap();
                    domo.color = value;
                    let color_raw = domo.color.raw();
                    match domo.peripheral.write_number(CMD_COLOR, 4, color_raw) {
                        Ok(_) => {}
                        Err(err) => println!("ERROR writing color: {}", err),
                    };
                }
                _ => {
                    println!("WARNING: unknown actuator: {}", name);
                }
            }
        } else {
            println!("UNKNOWN message: {:?}", &msg);
        }
    }
}

// Load configuration (name, serial number) to identify this controller to the server.
fn load_config() -> Config {
    let mut path = env::home_dir().expect("could not find home directory");
    path.push(CONFIG_PATH);
    // TODO error handling
    let f: fs::File = fs::File::open(path).expect("could not open config file");
    serde_json::from_reader(f).expect("could not parse config file")
}

// Loop endlessly and send sensor data to the server.
fn mainloop(peripheral: Peripheral) {
    env_logger::init().unwrap();

    let config = load_config();

    let (tx_msg_from_server, rx_msg_from_server): (Sender<MsgServer>, Receiver<MsgServer>) =
        channel();
    let (tx_msg_to_server, rx_msg_to_server): (Sender<String>, Receiver<String>) = channel();
    let tx_msg_to_server = Arc::new(Mutex::new(tx_msg_to_server));

    thread::spawn(move || {
        socket::Socket::connect(config, SERVER_URL, rx_msg_to_server, tx_msg_from_server);
    });

    let domo = Arc::new(Mutex::new(Domo {
        peripheral: peripheral,
        color: Color::new(),
    }));

    let tx_msg_to_server_clone = tx_msg_to_server.clone();
    let domo_clone = domo.clone();
    thread::spawn(move || {
        actuator_to_server(domo_clone, tx_msg_to_server_clone);
    });

    let domo_clone = domo.clone();
    thread::spawn(move || {
        msg_from_server(domo_clone, rx_msg_from_server);
    });

    println!("       Temperature:");
    log(domo.clone(), None);
    loop {
        let timestamp = Local::now().timestamp();
        let nextlog = timestamp / LOG_INTERVAL * LOG_INTERVAL + LOG_INTERVAL;
        thread::sleep(time::Duration::from_secs((nextlog - timestamp) as u64));
        log(domo.clone(), Some(tx_msg_to_server.clone()));
    }
}

fn main() {
    let mut peripheral = match Peripheral::open(SPIDEV_PATH) {
        Ok(peripheral) => peripheral,
        Err(err) => {
            println!("Could not open SPI device: {}", err);
            process::exit(1);
        }
    };

    // Parse param if it exists
    let param = match env::args().nth(2) {
        Some(strval) => {
            match u32::from_str_radix(strval.as_str(), 16) {
                Ok(val) => Some(val),
                Err(err) => {
                    println!("Could not parse argument \"{}\": {}", strval, err);
                    process::exit(1);
                }
            }
        }
        None => None,
    };

    match env::args().nth(1) {
        Some(ref cmd) if cmd == "resync" => {
            print!("resync:");
            for _ in 0..6 {
                match peripheral.resync() {
                    Ok(val) => print!(" {:02}", val),
                    Err(err) => {
                        println!(" error: {}", err);
                        process::exit(1);
                    }
                };
                thread::sleep(time::Duration::from_millis(100));
            }
            println!("");
        }
        Some(ref cmd) if cmd == "test2" || cmd == "test" => {
            match peripheral.read_number(CMD_TEST, 2) {
                Ok(val) => println!("test 2: {:04x}", val),
                Err(err) => println!("test 2: error: {}", err),
            };
        }
        Some(ref cmd) if cmd == "test4" => {
            match peripheral.read_number(CMD_TEST, 4) {
                Ok(val) => println!("test 4: {:08x}", val),
                Err(err) => println!("test 4: error: {}", err),
            };
        }
        Some(ref cmd) if cmd == "temp" || cmd == "temp-avg" => {
            match peripheral.read_number(CMD_TEMP_AVG, 2) {
                Ok(val) => println!("temp avg: {:.2}°C", decode_temp(val)),
                Err(err) => println!("temp avg: error: {}", err),
            };
        }
        Some(ref cmd) if cmd == "temp-now" => {
            match peripheral.read_number(CMD_TEMP_NOW, 2) {
                Ok(val) => println!("temp now: {:.2}°C", decode_temp(val)),
                Err(err) => println!("temp now: error: {}", err),
            };
        }
        Some(ref cmd) if cmd == "temp-raw" => {
            match peripheral.read_number(CMD_TEMP_RAW, 2) {
                Ok(val) => println!("temp raw: {} ({:.2}°C)", val, raw_to_celsius(val, 10)),
                Err(err) => println!("temp raw: error: {}", err),
            };
        }
        Some(ref cmd) if cmd == "color" => {
            match param {
                Some(param) => {
                    match peripheral.write_number(CMD_COLOR, 4, param) {
                        Ok(_) => {}
                        Err(err) => println!("ERROR writing color: {}", err),
                    };
                }
                None => {
                    match peripheral.read_number(CMD_COLOR, 4) {
                        Ok(val) => println!("color: {:08x}: {:?}", val, Color::from_raw(val)),
                        Err(err) => println!("color: error: {}", err),
                    };
                }
            };
        }
        Some(ref cmd) => {
            println!("unknown command: {}", cmd);
        }
        None => {
            mainloop(peripheral);
        }
    }
}
