
extern crate spidev;
use std::io::prelude::*;
use spidev::Spidev;
extern crate crc8;
use crc8::*;

use std::{env, io, thread, time};

const CMD_TEMP_NOW: u8 = 0x11;
const CMD_TEMP_AVG: u8 = 0x12;
const CMD_TEMP_RAW: u8 = 0x13;
const CMD_COLOR:    u8 = 0x05;
const CMD_TEST:     u8 = 0x20;

const TYPE_GETTER2: u8 = 0b00000000;
const TYPE_GETTER4: u8 = 0b01000000;

fn read_number(spi: &mut Spidev, cmd: u8, n: u8) -> Result<u32, io::Error> {
    let rawcmd = match n {
        2 => { cmd | TYPE_GETTER2 }
        4 => { cmd | TYPE_GETTER4 }
        _ => { panic!("n > 4 in read_number") }
    };

    try!(spi.write(&[rawcmd]));

    let mut buf: [u8; 5] = [0; 5];
    for i in 0..n as usize {
        thread::sleep(time::Duration::from_millis(1));
        try!(spi.read(&mut buf[i..i+1]));
    }

    thread::sleep(time::Duration::from_millis(1));
    try!(spi.read(&mut buf[n as usize..n as usize+1]));
    let crc = buf[n as usize];
    let crc2 = Crc8::create_lsb(0x82).calc(&buf, 2, 0);
    if crc == crc2 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "CRC check failed"));
    }

    let mut result: u32 = 0;
    for i in 0..n as usize {
        result >>= 8;
        let c = (buf[i] as u32) << ((n-1)*8);
        result += c;
    }

    Ok(result)
}

fn raw_to_celsius(value: u32, bits: u32) -> f64 {
    // TODO: these three constants should be read from the microcontroller
    let b_coefficient: f64 = 3950.0;  // β-coefficient
    let t0:            f64 = 298.15;  // nominal temperature (25°C)
    let r0:            f64 = 10000.0; // 10k at 25°C

    // convert value to range 0..1, where 0.5 means t=t0
    let fvalue: f64 = value as f64 / (1 << bits) as f64;

    // calculate resistance for NTC
    let r = r0 / (1.0 / fvalue - 1.0);

    // Steinhart-Hart equation
    let tinv = (1.0 / t0) + 1.0 / b_coefficient * (r / r0).ln();
    let t = 1.0 / tinv;

    // convert from K to °C
    return t - 273.15;
}

fn decode_temp(value: u32) -> f64 {
    // Value holds temperature in centidegrees, where 0 equals -55°C.
    // Convert this value to regular °C readings.
    return (value - 5500) as f64 / 100.0;
}

fn mainloop() {
    // TODO
}


fn main() {
    let mut spi = Spidev::open("/dev/spidev0.0").expect("could not open SPI device");

    match env::args().nth(1) {
        Some(ref cmd) if cmd == "test2" || cmd == "test" => {
            let result = read_number(&mut spi, CMD_TEST, 2).unwrap();
            println!("test 2: {:x}", result);
        },
        Some(ref cmd) if cmd == "test4" => {
            let result = read_number(&mut spi, CMD_TEST, 4).unwrap();
            println!("test 4: {:x}", result);
        },
        Some(ref cmd) if cmd == "temp" || cmd == "temp-avg" => {
            let result = read_number(&mut spi, CMD_TEMP_AVG, 2).unwrap();
            println!("temp avg: {:.2}°C", decode_temp(result));
        },
        Some(ref cmd) if cmd == "temp" || cmd == "temp-now" => {
            let result = read_number(&mut spi, CMD_TEMP_NOW, 2).unwrap();
            println!("temp now: {:.2}°C", decode_temp(result));
        },
        Some(ref cmd) if cmd == "temp-raw" => {
            let result = read_number(&mut spi, CMD_TEMP_RAW, 2).unwrap();
            println!("temp raw: {} ({:.2}°C)", result, raw_to_celsius(result, 10));
        },
        Some(ref cmd) if cmd == "color" => {
            let result = read_number(&mut spi, CMD_COLOR, 4).unwrap();
            println!("color: {:8x}", result);
        },
        Some(ref cmd) => {
            println!("unknown command: {}", cmd);
        },
        None => {
            mainloop();
        },
    }
}
