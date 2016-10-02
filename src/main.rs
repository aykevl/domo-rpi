
use std::{env, fs, process, thread, time};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Sender, Receiver};

extern crate chrono;
use chrono::*;

extern crate crc8;
extern crate spidev;

extern crate serde_json;
include!(concat!(env!("OUT_DIR"), "/messages.rs"));


extern crate ws;
extern crate env_logger;

mod peripheral;
use peripheral::*;


const CMD_TEMP_NOW: u8 = 0x11;
const CMD_TEMP_AVG: u8 = 0x12;
const CMD_TEMP_RAW: u8 = 0x13;
const CMD_COLOR: u8 = 0x05;
const CMD_TEST: u8 = 0x20;

const LOG_INTERVAL: i64 = 60 * 5; // 5 minutes
const SERVER_URL: &'static str = "wss://domo.aykevl.nl/api/ws/device";
const CONFIG_PATH: &'static str = ".config/domo.json";
const SPIDEV_PATH: &'static str = "/dev/spidev0.0";


fn raw_to_celsius(value: u32, bits: u32) -> f64 {
    // TODO: these three constants should be read from the microcontroller
    let b_coefficient: f64 = 3950.0;  // β-coefficient
    let t0: f64 = 298.15;  // nominal temperature (25°C)
    let r0: f64 = 10000.0; // 10k at 25°C

    // convert value to range 0..1, where 0.5 means t=t0
    let fvalue: f64 = value as f64 / (1 << bits) as f64;

    // calculate resistance for NTC
    let r = r0 / (1.0 / fvalue - 1.0);

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

fn log(mut peripheral: &mut Peripheral, tx_sensor: Option<&Sender<TemperatureLog>>) {
    let now = Local::now();
    let temp = match peripheral.read_number(CMD_TEMP_AVG, 2) {
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
    match tx_sensor {
        Some(tx_sensor) => {
            tx_sensor.send(TemperatureLog {
                    value: temp.unwrap(),
                    time: now.timestamp(),
                })
                .unwrap();
        }
        None => {}
    }
}

struct Socket {
    config: Config,
    rx_sensor: Arc<Mutex<Receiver<TemperatureLog>>>,
    verified_time: Arc<Mutex<bool>>,
}

impl Socket {
    fn run(&self, url: &str) -> ws::Result<()> {
        ws::connect(url, |out| {
            self.on_connect(&out);

            // Start thread that sends messages received via `rx_sensor`
            let verified_time = self.verified_time.clone();
            let rx_sensor_mutex = self.rx_sensor.clone();
            thread::spawn(move || {
                loop {
                    let rx_sensor = rx_sensor_mutex.lock().unwrap();
                    let msg = rx_sensor.recv().unwrap();

                    if !*verified_time.lock().unwrap() {
                        println!("Not verified time! I cannot make sure that the time on the \
                                  server and client is about the same.");
                        continue;
                    }

                    let msg_log = MsgSensorLog {
                        message: "sensorLog".to_string(),
                        name: "temp".to_string(),
                        value: msg.value,
                        time: msg.time,
                        _type: "temperature".to_string(),
                        interval: LOG_INTERVAL,
                    };
                    let msg_log_encoded = serde_json::to_string(&msg_log).unwrap();
                    match out.send(msg_log_encoded) {
                        Ok(_) => {}
                        Err(err) => {
                            println!("failed to send message: {}", err);
                            process::exit(1);
                        }
                    };
                }
            });

            move |msg_encoded| self.on_message(msg_encoded)
        })
    }

    fn on_connect(&self, out: &ws::Sender) {
        // send 'connect' message
        let msg_connect = MsgConnect {
            message: "connect".to_string(),
            name: self.config.name.clone(),
            serial: self.config.serial.clone(),
        };
        let msg_connect_encoded = serde_json::to_string(&msg_connect).unwrap();
        match out.send(msg_connect_encoded) {
            Ok(_) => {}
            Err(err) => {
                println!("failed to send message: {}", err);
                process::exit(1);
            }
        };
    }

    fn on_message(&self, msg_encoded: ws::Message) -> Result<(), ws::Error> {
        let msg_text = match msg_encoded {
            ws::Message::Text(val) => val,
            ws::Message::Binary(_) => {
                println!("received binary message");
                process::exit(1);
            }
        };
        let msg: MsgServer = match serde_json::from_str(&msg_text.as_str()) {
            Ok(msg) => msg,
            Err(err) => {
                println!("got invalid message from server: {}\nmessage: {}",
                         err,
                         &msg_text);
                process::exit(1);
            }
        };

        if msg.message == "time" {
            let timestamp = UTC::now().timestamp();
            match msg.value {
                Some(value) if (timestamp - value).abs() < 60 => {
                    let verified_time_mutex = self.verified_time.clone();
                    let mut verified_time = verified_time_mutex.lock().unwrap();
                    *verified_time = true;
                }
                Some(_) => {
                    println!("WARNING: time not in sync");
                }
                None => {
                    println!("WARNING: no timestamp sent in time message");
                }
            };
        } else {
            println!("UNKNOWN message: {}", &msg_text);
        }

        Ok(())
    }
}

struct TemperatureLog {
    value: f64,
    time: i64,
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
fn mainloop(mut peripheral: Peripheral) {
    env_logger::init().unwrap();

    let config = load_config();

    let (tx_sensor, rx_sensor): (Sender<TemperatureLog>, Receiver<TemperatureLog>) = channel();

    thread::spawn(move || {
        let socket = Socket {
            config: config,
            rx_sensor: Arc::new(Mutex::new(rx_sensor)),
            verified_time: Arc::new(Mutex::new(false)),
        };
        match socket.run(SERVER_URL) {
            Ok(_) => {}
            Err(err) => {
                println!("Could not open server socket: {}", err);
                process::exit(1);
            }
        };
    });

    println!("       Temperature:");
    log(&mut peripheral, None);
    loop {
        let timestamp = Local::now().timestamp();
        let nextlog = timestamp / LOG_INTERVAL * LOG_INTERVAL + LOG_INTERVAL;
        thread::sleep(time::Duration::from_secs((nextlog - timestamp) as u64));
        log(&mut peripheral, Some(&tx_sensor));
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
                    },
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
                        Ok(val) => println!("color: {:08x}", val),
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
