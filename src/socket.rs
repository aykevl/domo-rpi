
use std::{cmp, process, thread, time};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::Receiver;

use serde_json;
use ws;

use chrono::*;
use messages::*;

pub struct Socket {
    config: Config,
    rx_msg_to_server: Arc<Mutex<Receiver<String>>>,
    verified_time: Arc<Mutex<bool>>,
}

impl Socket {
    pub fn connect(config: Config, url: &str, rx_msg_to_server: Receiver<String>) {
        let socket = Socket {
            config: config,
            rx_msg_to_server: Arc::new(Mutex::new(rx_msg_to_server)),
            verified_time: Arc::new(Mutex::new(false)),
        };

        socket.run(url);
    }

    fn run(&self, url: &str) {
        let mut delay_seconds = 1;
        loop {
            match ws::connect(url, |out| {
                delay_seconds = 1;
                self.send_hello(&out);

                // Start thread that sends messages received via `rx_msg_to_server`
                let verified_time = self.verified_time.clone();
                let rx_msg_to_server_mutex = self.rx_msg_to_server.clone();
                thread::spawn(move || {
                    let rx_msg_to_server = rx_msg_to_server_mutex.lock().unwrap();
                    loop {
                        let msg = rx_msg_to_server.recv().unwrap();

                        if !*verified_time.lock().unwrap() {
                            println!("Not verified time! I cannot make sure that the time on the \
                                      server and client is about the same.");
                            continue;
                        }

                        match out.send(msg) {
                            Ok(_) => {}
                            Err(err) => {
                                // TODO: this drops a message. Don't do that.
                                println!("failed to send message, exiting thread: {}", err);
                                return;
                            }
                        };
                    }
                });

                move |msg_encoded| self.on_message(msg_encoded)
            }) {
                Ok(_) => {}
                Err(err) => {
                    delay_seconds = cmp::min(60, delay_seconds * 2);
                    println!("Could not open server socket (retrying in {}s): {}",
                             delay_seconds,
                             err);
                }
            };
            thread::sleep(time::Duration::from_secs(delay_seconds));
            println!("Reconnecting...");
        }
    }

    fn send_hello(&self, out: &ws::Sender) {
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
                return Ok(());
            }
        };

        if msg.message == "time" {
            let timestamp = UTC::now().timestamp();
            match msg.timestamp {
                Some(value) if (value - timestamp).abs() < 60 => {
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
        } else if msg.message == "actuator" {
            match msg.name {
                Some(name) => {
                    match &name[..] {
                        "color" => {
                            match msg.value {
                                Some(color) => {
                                    println!("color change from server: {:?}", color);
                                }
                                None => {
                                    println!("WARNING: no timestamp sent in time message");
                                }
                            }
                        }
                        _ => {
                            println!("WARNING: unknown actuator: {}", name);
                        }
                    }
                }
                None => {
                    println!("WARNING: no name sent with actuator message: {}", &msg_text);
                }
            }
        } else {
            println!("UNKNOWN message: {}", &msg_text);
        }

        Ok(())
    }
}
