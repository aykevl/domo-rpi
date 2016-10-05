
use std::io::prelude::*;
use std::{io, thread, time};

use spidev::Spidev;

use crc8::Crc8;


const TYPE_GETTER2: u8 = 0b00000000;
const TYPE_GETTER4: u8 = 0b01000000;
const TYPE_SETTER2: u8 = 0b10000000;
const TYPE_SETTER4: u8 = 0b11000000;


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

    pub fn resync(&mut self) -> Result<u8, io::Error> {
        let mut buf: [u8; 1] = [0; 1];
        match self.spi.write(&mut buf) {
            Ok(_) => Ok(buf[0]),
            Err(err) => Err(err),
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
            try!(self.spi.write(&mut buf[i..i + 1]));
        }

        Ok(())
    }
}
