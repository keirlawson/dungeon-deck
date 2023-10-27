use anyhow::bail;
use anyhow::Result;
use env_logger::Env;
use log::debug;
use log::info;
use paho_mqtt as mqtt;
use rodio::{Decoder, OutputStream, Sink};
use serde::Deserialize;
use std::convert::identity;
use std::fs;
use std::time::Duration;
use std::{fs::File, io::BufReader};
use streamdeck::DeviceImage;
use streamdeck::ImageOptions;
use streamdeck::{pids, Error, StreamDeck};

#[derive(Deserialize)]
struct BrokerConfig {
    host: String,
    user: String,
    pass: String,
}

#[derive(Deserialize)]
struct Button {
    image: String,
}

#[derive(Deserialize)]
struct Row {
    left: Option<Button>,
    middle: Option<Button>,
    right: Option<Button>,
}

#[derive(Deserialize)]
struct Buttons {
    top: Option<Row>,
    bottom: Option<Row>,
}

#[derive(Deserialize)]
struct Config {
    mqtt: Option<BrokerConfig>,
    buttons: Buttons,
}

fn populate_images(buttons: &mut Buttons, device: &StreamDeck) -> Result<Vec<Option<DeviceImage>>> {
    let top = buttons
        .top
        .as_mut()
        .map(|row| vec![row.left.take(), row.middle.take(), row.right.take()]);
    let bottom = buttons
        .bottom
        .as_mut()
        .map(|row| vec![row.left.take(), row.middle.take(), row.right.take()]);
    let paths = vec![top, bottom];
    paths
        .into_iter()
        .filter_map(identity)
        .flatten()
        .map(|opt| {
            opt.map(|path| {
                device
                    .load_image(&path.image, &ImageOptions::default())
                    .map_err(|e| e.into())
            })
            .transpose()
        })
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let config_contents = fs::read_to_string("./dungeon.toml").unwrap();
    let mut config: Config = toml::from_str(&config_contents).unwrap();

    if let Some(broker_config) = config.mqtt {
        let broker_url = format!("mqtt://{}", broker_config.host);

        let con_opts = mqtt::ConnectOptionsBuilder::new()
            .user_name(broker_config.user)
            .password(broker_config.pass)
            .finalize();

        let client = mqtt::AsyncClient::new(broker_url.as_str()).unwrap();

        client.connect(Some(con_opts)).await.unwrap();

        info!("Connected to MQTT broker at {}", broker_url);
    }

    const ELGATO_VID: u16 = 0x0fd9;
    let mut deck = StreamDeck::connect(ELGATO_VID, pids::REVISED_MINI, None).unwrap();

    info!("Connected to Stream Deck");
    let images = populate_images(&mut config.buttons, &deck)?;
    images
        .into_iter()
        .enumerate()
        .filter_map(move |(i, opt)| opt.map(|img| (i, img)))
        .map(|(i, img)| {
            let button_idx = (i + 1) as u8;
            debug!("Writing image to button {}", button_idx);
            deck.write_button_image(button_idx, &img)
                .map_err(|e| e.into())
        })
        .collect::<Result<()>>()?;

    const POLL_WAIT: Duration = Duration::new(1, 0);
    loop {
        let result = deck.read_buttons(Some(POLL_WAIT));
        match result {
            Ok(pressed) => handle_press(&pressed),
            Err(err) => {
                if !matches!(err, Error::NoData) {
                    bail!(err)
                }
            }
        }
    }
}

fn handle_press(pressed: &Vec<u8>) {
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let file = BufReader::new(File::open("epicbattle.mp3").unwrap());
    let source = Decoder::new(file).unwrap();
    let sink = Sink::try_new(&stream_handle).unwrap();
    sink.append(source);
    sink.sleep_until_end();
}
