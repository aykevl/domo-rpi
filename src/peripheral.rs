
use std::io::prelude::*;
use std::{io, thread, time};

use spidev::{Spidev, SpidevTransfer};

use crc8::Crc8;


const TYPE_GETTER2: u8 = 0b00000000;
const TYPE_GETTER4: u8 = 0b01000000;
const TYPE_SETTER2: u8 = 0b10000000;
const TYPE_SETTER4: u8 = 0b11000000;

pub const CMD_COLOR: u8 = 0x05;
pub const CMD_TEMP_NOW: u8 = 0x11; // current temp (calculated on AVR)
pub const CMD_TEMP_AVG: u8 = 0x12; // average temp (calculated on AVR)
pub const CMD_TEMP_RAW: u8 = 0x13; // raw temp sensor reading
pub const CMD_TEMP_RSUM: u8 = 0x14; // sum of 256 raw temp sensor readings
pub const CMD_TEMP_SRES: u8 = 0x15; // constant: series resistor
pub const CMD_TEMP_NRES: u8 = 0x16; // constant: NTC resistor at 25°C
pub const CMD_TEMP_BCOE: u8 = 0x17; // constant: NTC β-coefficient
pub const CMD_TEST: u8 = 0x20;

pub struct Peripheral {
    spi: Spidev,
    crc8: Crc8,
}

impl Peripheral {
    pub fn open(path: &str) -> Result<Peripheral, io::Error> {
        Ok(Peripheral {
            spi: try!(Spidev::open(path)),
            crc8: Crc8::create_msb(0x07),
        })
    }

    pub fn resync(&mut self) -> Result<(), io::Error> {
        let cmd = TYPE_GETTER2 | CMD_TEST;
        try!(self.spi.write(&[cmd]));

        // read until start-of-command
        loop {
            thread::sleep(time::Duration::from_millis(1));
            let mut transfer = SpidevTransfer::write(&[cmd]);
            try!(self.spi.transfer(&mut transfer));
            // start of command
            if transfer.rx_buf.unwrap()[0] == 0xff {
                break;
            }
        }

        // read rest of command
        let mut buf: [u8; 3] = [0; 3];
        for i in 0..3 as usize {
            thread::sleep(time::Duration::from_millis(1));
            try!(self.spi.read(&mut buf[i..i + 1]));
        }

        // is this the correct response?
        if &buf[..] == [0xcd, 0xab, 0x1f] {
            Ok(())
        } else {
            let err_str = format!("expected cdab1f in resync, got {:02x}{:02x}{:02x}",
                                  buf[0],
                                  buf[1],
                                  buf[2]);
            Err(io::Error::new(io::ErrorKind::InvalidData, err_str))
        }
    }

    pub fn read_number(&mut self, cmd: u8, length: u8) -> Result<u32, io::Error> {
        let rawcmd = match length {
            2 => cmd | TYPE_GETTER2,
            4 => cmd | TYPE_GETTER4,
            _ => panic!("length is not 2 or 4 in read_number"),
        };

        thread::sleep(time::Duration::from_millis(1));
        try!(self.spi.write(&[rawcmd]));

        let mut buf: [u8; 1] = [0; 1];
        thread::sleep(time::Duration::from_millis(1));
        try!(self.spi.read(&mut buf));
        if buf[0] != 0xff {
            let err_string = format!("expected 0xff from SPI, got {}", buf[0]);
            return Err(io::Error::new(io::ErrorKind::InvalidData, err_string));
        }

        let mut buf: [u8; 6] = [0; 6];
        buf[0] = rawcmd;
        for i in 0..length as usize + 1 {
            thread::sleep(time::Duration::from_millis(1));
            try!(self.spi.read(&mut buf[i + 1..i + 2]));
        }

        let crc = buf[length as usize + 1];
        let crc2 = self.crc8.calc(&buf, length as i32 + 1, 0);
        if crc != crc2 {
            print!("checksum problem (received {:02x}, calculated {:02x}) for message",
                   crc,
                   crc2);
            for c in &buf[0 + 1..length as usize + 1] {
                print!(" {:02x}", c);
            }
            println!("");
            return Err(io::Error::new(io::ErrorKind::InvalidData, "CRC check failed"));
        }

        let mut result: u32 = 0;
        for i in 0..length as usize {
            result >>= 8;
            let c = (buf[i + 1] as u32) << ((length - 1) * 8);
            result += c;
        }

        Ok(result)
    }

    pub fn write_number(&mut self, cmd: u8, length: u8, value: u32) -> Result<(), io::Error> {
        let rawcmd = match length {
            2 => cmd | TYPE_SETTER2,
            4 => cmd | TYPE_SETTER4,
            _ => panic!("length is not 2 or 4 in write_number"),
        };

        let mut buf: [u8; 6] = [0; 6];
        buf[0] = rawcmd;
        let mut value2 = value;
        for i in 0..length as usize {
            buf[i + 1] = (value2 % 256) as u8;
            value2 /= 256;
        }
        let crc = Crc8::create_msb(0x07).calc(&buf, length as i32 + 1, 0);
        buf[length as usize + 1] = crc;

        for i in 0..length as usize + 2 {
            thread::sleep(time::Duration::from_millis(1));
            try!(self.spi.write(&buf[i..i + 1]));
        }

        Ok(())
    }
}
