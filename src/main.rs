
use std::{env, fs, io, process, thread, time};
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


const LOG_INTERVAL: i64 = 60 * 5; // 5 minutes
const SERVER_URL: &'static str = "wss://domo.aykevl.nl/api/ws/device";
const CONFIG_PATH: &'static str = ".config/domo.json";
const SPIDEV_PATH: &'static str = "/dev/spidev0.0";
const COLOR_READ_TIMEOUT: u64 = 5; // 5 seconds


fn decode_temp(value: u32) -> f64 {
    // Value holds temperature in centidegrees, where 0 equals -55°C.
    // Convert this value to regular °C readings.
    ((value as i32 - 5500) as f64) / 100.0
}

struct Domo {
    config: Config,
    peripheral: Peripheral,
    color: Color,
    temp_b_coefficient: Option<f64>,
    temp_nominal_r: Option<f64>,
    temp_series_resistor: Option<f64>,
}

impl Domo {
    fn new(spidev_path: &str) -> Result<Self, io::Error> {
        let peripheral = match Peripheral::open(spidev_path) {
            Ok(peripheral) => peripheral,
            Err(err) => {
                println!("Could not open SPI device: {}", err);
                process::exit(1);
            }
        };

        // Load configuration (name, serial number) to identify this controller to the server.
        // TODO error handling
        let mut path = env::home_dir().expect("could not find home directory");
        path.push(CONFIG_PATH);
        let f: fs::File = fs::File::open(path).expect("could not open config file");
        let config = serde_json::from_reader(f).expect("could not parse config file");

        Ok(Domo {
            config: config,
            peripheral: peripheral,
            color: Color::new(),
            temp_b_coefficient: None,
            temp_nominal_r: None,
            temp_series_resistor: None,
        })
    }

    fn get_name(&self) -> String {
        return self.config.name.clone();
    }

    fn get_serial(&self) -> String {
        return self.config.serial.clone();
    }

    fn resync(&mut self) -> Result<(), io::Error> {
        self.peripheral.resync()
    }

    fn read_number(&mut self, cmd: u8, length: u8) -> Result<u32, io::Error> {
        self.peripheral.read_number(cmd, length)
    }

    fn write_number(&mut self, cmd: u8, length: u8, value: u32) -> Result<(), io::Error> {
        self.peripheral.write_number(cmd, length, value)
    }

    fn read_temp_raw(&mut self) -> Result<f64, io::Error> {
        let raw_value = try!(self.peripheral.read_number(CMD_TEMP_RAW, 4));
        self.raw_to_celsius(raw_value, 10)
    }

    fn read_temp_rsum(&mut self) -> Result<f64, io::Error> {
        let raw_value = try!(self.peripheral.read_number(CMD_TEMP_RSUM, 4));
        self.raw_to_celsius(raw_value, 18)
    }

    fn get_temp_b_coefficient(&mut self) -> Result<f64, io::Error> {
        Ok(match self.config.temp_b_coefficient {
            Some(val) => val,
            None => match self.temp_b_coefficient {
                Some(val) => val,
                None => {
                    let b_coefficient = try!(self.peripheral.read_number(CMD_TEMP_BCOE, 2)) as f64;
                    self.temp_b_coefficient = Some(b_coefficient);
                    b_coefficient // return
                }
            }
        })
    }

    fn get_temp_nominal_r(&mut self) -> Result<f64, io::Error> {
        Ok(match self.config.temp_nominal_r {
            Some(val) => val,
            None => match self.temp_nominal_r {
                Some(val) => val,
                None => {
                    let nominal_r = try!(self.peripheral.read_number(CMD_TEMP_NRES, 2)) as f64;
                    self.temp_nominal_r = Some(nominal_r);
                    nominal_r // return
                }
            }
        })
    }

    fn get_temp_series_resistor(&mut self) -> Result<f64, io::Error> {
        Ok(match self.config.temp_nominal_r {
            Some(val) => val,
            None => match self.temp_series_resistor {
                Some(val) => val,
                None => {
                    let series_resistor = try!(self.peripheral.read_number(CMD_TEMP_SRES, 2)) as f64;
                    self.temp_nominal_r = Some(series_resistor);
                    series_resistor // return
                }
            }
        })
    }

    fn raw_to_celsius(&mut self, value: u32, bits: u32) -> Result<f64, io::Error> {
        // Source: https://learn.adafruit.com/thermistor/using-a-thermistor
        // TODO: these constants should be read from the microcontroller
        let b_coefficient = try!(self.get_temp_b_coefficient());
        let t0: f64 = 298.15;  // nominal temperature (25°C)
        let r0: f64 = try!(self.get_temp_nominal_r()); // 10kΩ at 25°C
        let series_resistor: f64 = try!(self.get_temp_series_resistor()); // 10kΩ series resistor

        // convert value to range 0..1, where 0.5 means t=t0
        let fvalue: f64 = value as f64 / (1 << bits) as f64;

        // calculate resistance for NTC
        let r = series_resistor / (1.0 / fvalue - 1.0);

        // Steinhart-Hart equation
        let tinv = (1.0 / t0) + 1.0 / b_coefficient * (r / r0).ln();
        let t = 1.0 / tinv;

        // convert from K to °C and return
        Ok(t - 273.15)
    }
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

// Loop endlessly and send sensor data to the server.
fn mainloop(domo: Domo) {
    env_logger::init().unwrap();

    let (tx_msg_from_server, rx_msg_from_server): (Sender<MsgServer>, Receiver<MsgServer>) =
        channel();
    let (tx_msg_to_server, rx_msg_to_server): (Sender<String>, Receiver<String>) = channel();
    let tx_msg_to_server = Arc::new(Mutex::new(tx_msg_to_server));

    let name = domo.get_name();
    let serial = domo.get_serial();
    thread::spawn(move || {
        socket::Socket::connect(SERVER_URL, name, serial, rx_msg_to_server, tx_msg_from_server);
    });

    // enable locking
    let domo = Arc::new(Mutex::new(domo));

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
    let mut domo = match Domo::new(SPIDEV_PATH) {
        Ok(val) => val,
        Err(err) => {
            println!("error: {}", err);
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
            print!("resync: ");
            match domo.resync() {
                Ok(_) => println!("done"),
                Err(err) => {
                    println!(" error: {}", err);
                    process::exit(1);
                }
            };
        }
        Some(ref cmd) if cmd == "test2" || cmd == "test" => {
            match domo.read_number(CMD_TEST, 2) {
                Ok(val) => println!("test 2: {:04x}", val),
                Err(err) => println!("test 2: error: {}", err),
            };
        }
        Some(ref cmd) if cmd == "test4" => {
            match domo.read_number(CMD_TEST, 4) {
                Ok(val) => println!("test 4: {:08x}", val),
                Err(err) => println!("test 4: error: {}", err),
            };
        }
        Some(ref cmd) if cmd == "temp" || cmd == "temp-avg" => {
            match domo.read_number(CMD_TEMP_AVG, 2) {
                Ok(val) => println!("temp avg: {:.2}°C", decode_temp(val)),
                Err(err) => println!("temp avg: error: {}", err),
            };
        }
        Some(ref cmd) if cmd == "temp-now" => {
            match domo.read_number(CMD_TEMP_NOW, 2) {
                Ok(val) => println!("temp now: {:.2}°C", decode_temp(val)),
                Err(err) => println!("temp now: error: {}", err),
            };
        }
        Some(ref cmd) if cmd == "temp-rsum" => {
            match domo.read_temp_rsum() {
                Ok(val) => println!("temp rsum: {:.2}°C", val),
                Err(err) => println!("temp rsum: error: {}", err),
            };
        }
        Some(ref cmd) if cmd == "temp-raw" => {
            match domo.read_temp_raw() {
                Ok(val) => println!("temp raw: {:.2}°C", val),
                Err(err) => println!("temp raw: error: {}", err),
            };
        }
        Some(ref cmd) if cmd == "color" => {
            match param {
                Some(param) => {
                    match domo.write_number(CMD_COLOR, 4, param) {
                        Ok(_) => {}
                        Err(err) => println!("ERROR writing color: {}", err),
                    };
                }
                None => {
                    match domo.read_number(CMD_COLOR, 4) {
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
            mainloop(domo);
        }
    }
}
