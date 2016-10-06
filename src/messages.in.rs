
// Message received from server
#[derive(Deserialize)]
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
}

// Send color to server
#[derive(Serialize)]
pub struct MsgColor {
    pub message: String,
    pub name: String,
    pub value: Color,
}

#[derive(Serialize,Deserialize,Debug,Default)]
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
    pub fn from_raw(value: u32) -> Color {
        let mut bytes: [u8; 4] = [0; 4];
        let mut raw_value = value;
        for i in 0..4 {
            bytes[i] = ((raw_value & 0xff000000) >> 24) as u8;
            raw_value <<= 8;
        }

        let mut color = Color{..Default::default()};

        let mode = bytes[0] & COLOR_MODE_MASK;
        color.mode = match mode {
            COLOR_MODE_RGB => "rgb",
            COLOR_MODE_HSV => "hsv",
            COLOR_MODE_HSV_MAX => "hsv-max",
            COLOR_MODE_UNDEF1 => "undefined-1",
            _ => panic!("unreachable"),
        }.to_string();

        color.is_white = (mode & COLOR_FLAG_WHITE) != 0;
        color.is_looping = (mode & COLOR_FLAG_LOOPING) != 0;

        if mode == COLOR_MODE_HSV || mode == COLOR_MODE_HSV_MAX {
            if color.is_looping {
                color.time = ufloat8::decode(bytes[1]) as f32 / 4.0;
            } else {
                color.hue = bytes[1] as f32 / 255.0;
            }
            color.saturation = bytes[2] as f32 / 255.0;
            color.value = bytes[3] as f32 / 255.0;
        } else {
            color.red = bytes[1] as f32 / 255.0;
            color.green = bytes[2] as f32 / 255.0;
            color.blue = bytes[3] as f32 / 255.0;
        }

        // return the color
        color
    }
}
