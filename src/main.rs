use anyhow::bail;
use anyhow::Result;
use env_logger::Env;
use log::debug;
use log::info;
use mqtt::Client;
use paho_mqtt as mqtt;
use rodio::{Decoder, OutputStream, Sink};
use serde::Deserialize;
use std::collections::HashMap;
use std::convert::identity;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use std::{fs::File, io::BufReader};
use streamdeck::DeviceImage;
use streamdeck::ImageOptions;
use streamdeck::{pids, Error, StreamDeck};

struct Button {
    config: ButtonConfig,
    playing: bool,
}

#[derive(Deserialize)]
struct BrokerConfig {
    host: String,
    user: String,
    pass: String,
}

#[derive(Deserialize, Clone)]
struct ButtonConfig {
    image: Option<String>,
    sound: Option<PathBuf>,
    topic: Option<String>,
    payload: Option<String>,
}

#[derive(Deserialize)]
struct Row {
    left: Option<ButtonConfig>,
    middle: Option<ButtonConfig>,
    right: Option<ButtonConfig>,
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

fn list_buttons(buttons: &mut Buttons) -> Vec<Option<ButtonConfig>> {
    let top = buttons
        .top
        .as_mut()
        .map(|row| vec![row.left.take(), row.middle.take(), row.right.take()]);
    let bottom = buttons
        .bottom
        .as_mut()
        .map(|row| vec![row.left.take(), row.middle.take(), row.right.take()]);
    let list = vec![top, bottom];
    list.into_iter().filter_map(identity).flatten().collect()
}

fn connect_mqtt(config: BrokerConfig) -> Result<Client> {
    let broker_url = format!("mqtt://{}", config.host);

    let con_opts = mqtt::ConnectOptionsBuilder::new()
        .user_name(config.user)
        .password(config.pass)
        .finalize();

    let client = mqtt::Client::new(broker_url.as_str())?;

    client.connect(Some(con_opts))?;

    info!("Connected to MQTT broker at {}", broker_url);

    Ok(client)
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    let kill = Arc::new(AtomicBool::new(false));
    let k2 = kill.clone();
    ctrlc::set_handler(move || k2.store(true, Ordering::Relaxed)).unwrap();

    let config_contents = fs::read_to_string("./dungeon.toml")?;
    let mut config: Config = toml::from_str(&config_contents)?;

    //FIXME can we do this with a map now?
    let broker_client = if let Some(client) = config.mqtt.map(|c| connect_mqtt(c)) {
        let client = client?;
        Some(client)
    } else {
        None
    };

    const ELGATO_VID: u16 = 0x0fd9;
    let mut deck = StreamDeck::connect(ELGATO_VID, pids::REVISED_MINI, None)?;
    info!("Connected to Stream Deck");

    let buttons = list_buttons(&mut config.buttons);
    let images: Result<Vec<Option<DeviceImage>>> = buttons
        .clone()
        .into_iter()
        .map(|opt| {
            opt.and_then(|conf| {
                conf.image.map(|path| {
                    deck.load_image(&path, &ImageOptions::default())
                        .map_err(|e| e.into())
                })
            })
            .transpose()
        })
        .collect();
    let images = images?;
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
    let mut button_state: HashMap<usize, Button> = buttons
        .into_iter()
        .map(|conf| Button {
            //FIXME impl default
            config: conf.unwrap_or(ButtonConfig {
                image: None,
                sound: None,
                topic: None,
                payload: None,
            }),
            playing: false,
        })
        .enumerate()
        .collect();

    let (_stream, stream_handle) = OutputStream::try_default()?;
    let sink = Sink::try_new(&stream_handle)?;
    info!("Acquired audio sink");

    const POLL_WAIT: Duration = Duration::from_millis(250);
    loop {
        let result = deck.read_buttons(Some(POLL_WAIT));
        match result {
            Ok(pressed) => handle_press(&sink, &mut button_state, &pressed, &broker_client)?,
            Err(err) => {
                if !matches!(err, Error::NoData) {
                    bail!(err)
                }
            }
        }
        if kill.load(Ordering::Relaxed) == true {
            if let Some(cli) = broker_client {
                cli.disconnect(None).unwrap();
            }
            sink.stop();
            debug!("Exiting gracefully");
            break Ok(());
        }
    }
}

fn pressed_idx(states: &Vec<u8>) -> Option<usize> {
    states
        .iter()
        .enumerate()
        .find(|(_, state)| state == &&1)
        .map(|(i, _)| i)
}

fn handle_press(
    sink: &Sink,
    buttons: &mut HashMap<usize, Button>,
    pressed: &Vec<u8>,
    mqtt: &Option<Client>,
) -> Result<()> {
    if let Some(idx) = pressed_idx(pressed) {
        debug!("Button {} pressed", idx);
        let button = buttons.get_mut(&idx).unwrap();
        if button.playing {
            sink.stop();
            button.playing = false;
        } else {
            if let Some(path) = &button.config.sound {
                let file = BufReader::new(File::open(path)?);
                let source = Decoder::new(file)?;
                sink.empty();
                debug!("Playing audio file {:?}", path);
                sink.append(source);
                button.playing = true;
            }
        }

        if let (Some(mqtt), Some(topic), Some(payload)) =
            (mqtt, &button.config.topic, &button.config.payload)
        {
            let message = mqtt::Message::new(topic, payload.as_str(), mqtt::QOS_0);
            debug!(
                "Sending message to topic {} with payload {}",
                topic, payload
            );
            mqtt.publish(message)?;
        }
    } else {
        debug!("Buttons released");
    }
    Ok(())
}
