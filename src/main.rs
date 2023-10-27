use paho_mqtt as mqtt;
use rodio::{Decoder, OutputStream, Sink};
use serde::Deserialize;
use std::fs;
use std::{fs::File, io::BufReader};

#[derive(Deserialize)]
struct BrokerConfig {
    host: String,
    user: String,
    pass: String,
}

#[derive(Deserialize)]
struct Config {
    mqtt: Option<BrokerConfig>,
}

#[tokio::main]
async fn main() {
    let config_contents = fs::read_to_string("./dungeon.toml").unwrap();
    let config: Config = toml::from_str(&config_contents).unwrap();

    if let Some(broker_config) = config.mqtt {
        let broker_url = format!("mqtt://{}", broker_config.host);

        let con_opts = mqtt::ConnectOptionsBuilder::new()
            .user_name(broker_config.user)
            .password(broker_config.pass)
            .finalize();

        let client = mqtt::AsyncClient::new(broker_url).unwrap();

        client.connect(Some(con_opts)).await.unwrap();

        println!("Broker connected");
    }

    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let file = BufReader::new(File::open("epicbattle.mp3").unwrap());
    let source = Decoder::new(file).unwrap();
    let sink = Sink::try_new(&stream_handle).unwrap();
    sink.append(source);
    sink.sleep_until_end();

    println!("Hello, world!");
}
