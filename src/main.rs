use std::{fs::File, io::BufReader};

use paho_mqtt as mqtt;
use rodio::{Decoder, OutputStream, Sink};

#[tokio::main]
async fn main() {
    //FIXME take these at runtime/from config
    let broker_url = format!("mqtt://{}", env!("MQTT_HOST"));
    let broker_user = env!("MQTT_USER");
    let broker_pass = env!("MQTT_PASS");

    let con_opts = mqtt::ConnectOptionsBuilder::new()
        .user_name(broker_user)
        .password(broker_pass)
        .finalize();

    let client = mqtt::AsyncClient::new(broker_url).unwrap();

    client.connect(Some(con_opts)).await.unwrap();

    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let file = BufReader::new(File::open("epicbattle.mp3").unwrap());
    let source = Decoder::new(file).unwrap();
    let sink = Sink::try_new(&stream_handle).unwrap();
    sink.append(source);
    sink.sleep_until_end();

    println!("Hello, world!");
}
