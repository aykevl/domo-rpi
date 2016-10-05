
// Message received from server
#[derive(Deserialize)]
struct MsgServer {
    message: String,
    timestamp: Option<i64>,
}


// Connect message from client to server
#[derive(Serialize)]
struct MsgConnect {
    message: String,
    name: String,
    serial: String,
}

// Send temperature to server
#[derive(Serialize)]
struct MsgSensorLog {
    message: String,
    name: String,
    value: f64,
    time: i64,
    #[serde(rename="type")]
    sensor_type: String,
    interval: i64,
}

// Config data
#[derive(Serialize, Deserialize)]
struct Config {
    name: String,
    serial: String,
}
