
// Message received from server
#[derive(Deserialize,Debug)]
pub struct MsgServer {
    pub message: String,
    pub name: Option<String>,
    pub timestamp: Option<i64>,
    pub value: Option<Color>,
}

// Connect message from client to server
#[derive(Serialize)]
pub struct MsgConnect {
    pub message: String,
    pub name: String,
    pub serial: String,
}

// Send temperature to server
#[derive(Serialize)]
pub struct MsgSensorLog {
    pub message: String,
    pub name: String,
    pub value: f64,
    pub time: i64,
    #[serde(rename="type")]
    pub sensor_type: String,
    pub interval: i64,
}

// Config data
#[derive(Serialize, Deserialize)]
pub struct Config {
    pub name: String,
    pub serial: String,
    pub temp_b_coefficent: Option<f64>,
    pub temp_nominal_r: Option<f64>,
    pub temp_series_resistor: Option<f64>,
}

// Send color to server
#[derive(Serialize)]
pub struct MsgColor {
    pub message: String,
    pub name: String,
    pub value: Color,
}

#[derive(Serialize,Deserialize,Debug,Default,Clone)]
pub struct Color {
    mode: String,
    #[serde(rename="isWhite")]
    is_white: bool,
    #[serde(rename="looping")]
    is_looping: bool,
    hue: f32,
    time: f32,
    saturation: f32,
    value: f32,
    red: f32,
    green: f32,
    blue: f32,
}

// these should be const members of the Color impl
const COLOR_FLAG_WHITE: u8 = 0b10000000;
const COLOR_FLAG_LOOPING: u8 = 0b01000000;
const COLOR_MODE_MASK: u8 = 0b00000011;
const COLOR_MODE_RGB: u8 = 0b00000000;
const COLOR_MODE_HSV: u8 = 0b00000001;
const COLOR_MODE_HSV_MAX: u8 = 0b00000010;
const COLOR_MODE_UNDEF1: u8 = 0b00000011;

impl Color {
    pub fn new() -> Self {
        Color{..Default::default()}
    }

    pub fn from_raw(value: u32) -> Self {
        let mut color = Color::new();
        color.update(value);

        // Return the color
        color
    }

    pub fn update(&mut self, value: u32) {
        let mut bytes: [u8; 4] = [0; 4];
        let mut raw_value = value;
        for i in 0..4 {
            bytes[i] = ((raw_value & 0xff000000) >> 24) as u8;
            raw_value <<= 8;
        }

        let mode = bytes[0] & COLOR_MODE_MASK;
        self.mode = match mode {
            COLOR_MODE_RGB => "rgb",
            COLOR_MODE_HSV => "hsv",
            COLOR_MODE_HSV_MAX => "hsv-max",
            COLOR_MODE_UNDEF1 => "undefined-1",
            _ => panic!("unreachable"),
        }.to_string();

        self.is_white = (bytes[0] & COLOR_FLAG_WHITE) != 0;
        self.is_looping = (bytes[0] & COLOR_FLAG_LOOPING) != 0;

        if mode == COLOR_MODE_RGB {
            self.red = bytes[1] as f32 / 255.0;
            self.green = bytes[2] as f32 / 255.0;
            self.blue = bytes[3] as f32 / 255.0;
        } else if mode == COLOR_MODE_HSV || mode == COLOR_MODE_HSV_MAX {
            if self.is_looping {
                self.time = ufloat8::decode(bytes[1]) as f32 / 4.0;
            } else {
                self.hue = bytes[1] as f32 / 255.0;
            }
            self.saturation = bytes[2] as f32 / 255.0;
            self.value = bytes[3] as f32 / 255.0;
        } else {
            // Unknown mode, we can't know what those bits mean.
        }
    }

    pub fn raw(&self) -> u32 {
        let mut bytes: [u8; 4] = [0; 4];

        bytes[0] = match self.mode.as_str() {
            "rgb" => COLOR_MODE_RGB,
            "hsv" => COLOR_MODE_HSV,
            "hsv-max" => COLOR_MODE_HSV_MAX,
            _ => COLOR_MODE_UNDEF1,
        };
        if self.is_white {
            bytes[0] |= COLOR_FLAG_WHITE;
        }
        if self.is_looping {
            bytes[0] |= COLOR_FLAG_LOOPING;
        }

        if self.mode == "rgb" {
            bytes[1] = (self.red * 255.0).round() as u8;
            bytes[2] = (self.green * 255.0).round() as u8;
            bytes[3] = (self.blue * 255.0).round() as u8;
        } else if self.mode == "hsv" || self.mode == "hsv-max" {
            if self.is_looping {
                bytes[1] = ufloat8::encode((self.time*4.0).round() as u32);
            } else {
                bytes[1] = (self.hue * 255.0).round() as u8;
            }

            bytes[2] = (self.saturation * 255.0).round() as u8;
            bytes[3] = (self.value * 255.0).round() as u8;
        } else {
            // Unknown mode, we can't know what those bytes should contain.
        }

        let mut raw: u32 = 0;
        for i in 0..4 {
            raw = (raw << 8) | bytes[i] as u32;
        }

        // return the raw value
        raw
    }
}
