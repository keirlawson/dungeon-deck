use anyhow::bail;
use anyhow::Result;
use env_logger::Env;
use image::imageops;
use image::imageops::FilterType;
use image::io::Reader;
use image::DynamicImage;
use log::debug;
use log::error;
use log::info;
use mqtt::Client;
use paho_mqtt as mqtt;
use rodio::{Decoder, OutputStream, Sink};
use serde::Deserialize;
use std::collections::HashMap;
use std::convert::identity;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use std::{fs::File, io::BufReader};
use streamdeck::{pids, Error, StreamDeck};
const PLAY_IMG: &[u8] = include_bytes!("../img/play.png");
const STOP_IMG: &[u8] = include_bytes!("../img/stop.png");
const DEFAULT_CONFIG_LOCATION: &str = "./dungeon.toml";

struct Button {
    config: ButtonConfig,
    playing: bool,
    image: Option<DynamicImage>,
}

impl Default for Button {
    fn default() -> Self {
        Button {
            config: ButtonConfig {
                image: None,
                sound: None,
                topic: None,
                payload: None,
            },
            playing: false,
            image: None,
        }
    }
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
    #[serde(default)]
    playicon: bool,
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
        .automatic_reconnect(Duration::from_secs(1), Duration::from_secs(15))
        .finalize();

    let client = mqtt::Client::new(broker_url.as_str())?;

    client.connect(Some(con_opts))?;

    info!("Connected to MQTT broker at {}", broker_url);

    Ok(client)
}

fn write_images(
    state: &HashMap<usize, Button>,
    deck: &mut StreamDeck,
    play_img: &DynamicImage,
    show_play_icon: bool,
) -> Result<()> {
    //FIXME still render initial play button when image not set
    state
        .iter()
        .filter_map(|(i, but)| {
            but.image
                .as_ref()
                .map(|img| (i, img, but.config.sound.is_some()))
        })
        .map(|(i, img, is_audio)| {
            let mut img = img.clone();
            if is_audio && show_play_icon {
                imageops::overlay(&mut img, play_img, 0, 0);
            }
            write_image(i, deck, img)
        })
        .collect::<Result<()>>()
}

fn write_image(
    idx: &usize,
    deck: &mut StreamDeck,
    img: DynamicImage,
) -> std::result::Result<(), anyhow::Error> {
    let button_idx = (idx + 1) as u8;
    debug!("Writing image to button {}", button_idx);
    deck.set_button_image(button_idx, img).map_err(|e| e.into())
}

fn write_overlayed(
    idx: usize,
    img: &DynamicImage,
    overlay_img: &DynamicImage,
    deck: &mut StreamDeck,
) -> Result<()> {
    let mut img = img.clone();
    imageops::overlay(&mut img, overlay_img, 0, 0);
    write_image(&idx, deck, img)
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    let kill = Arc::new(AtomicBool::new(false));
    let k2 = kill.clone();
    ctrlc::set_handler(move || k2.store(true, Ordering::Relaxed)).unwrap();

    let args: Vec<String> = env::args().collect();
    let config_contents = if let Some(location) = args.get(1) {
        fs::read_to_string(location)?
    } else {
        fs::read_to_string(DEFAULT_CONFIG_LOCATION)?
    };

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

    let (width, height) = deck.kind().image_size();
    let play_img = image::load_from_memory(PLAY_IMG)?;
    let stop_img = image::load_from_memory(STOP_IMG)?;
    let buttons = list_buttons(&mut config.buttons);
    let mut button_state = build_state(buttons, width, height)?;
    write_images(&button_state, &mut deck, &play_img, config.playicon)?;

    let (_stream, stream_handle) = OutputStream::try_default()?;
    let sink = Sink::try_new(&stream_handle)?;
    info!("Acquired audio sink");

    const POLL_WAIT: Duration = Duration::from_millis(250);
    loop {
        let result = deck.read_buttons(Some(POLL_WAIT));
        match result {
            Ok(pressed) => handle_press(
                &sink,
                &mut button_state,
                &pressed,
                &broker_client,
                &play_img,
                &stop_img,
                &mut deck,
                config.playicon,
            )?,
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

fn build_state(
    buttons: Vec<Option<ButtonConfig>>,
    width: usize,
    height: usize,
) -> Result<HashMap<usize, Button>> {
    buttons
        .into_iter()
        .map(move |conf| {
            let button = if let Some(conf) = conf {
                let image = conf
                    .image
                    .as_ref()
                    .map(|path| {
                        let image = Reader::open(&path)?.decode()?;
                        let image = image.resize(width as u32, height as u32, FilterType::Gaussian);
                        anyhow::Ok(image)
                    })
                    .transpose()?;

                Button {
                    config: conf,
                    playing: false,
                    image,
                }
            } else {
                Button::default()
            };
            Ok(button)
        })
        .enumerate()
        .map(|(i, r)| r.map(|c| (i, c)))
        .collect::<Result<HashMap<usize, Button>>>()
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
    play_img: &DynamicImage,
    stop_img: &DynamicImage,
    deck: &mut StreamDeck,
    show_play_icon: bool,
) -> Result<()> {
    if let Some(idx) = pressed_idx(pressed) {
        debug!("Button {} pressed", idx);
        let button = buttons.get_mut(&idx).unwrap();
        if button.playing {
            sink.stop();
            button.playing = false;
            if let Some(img) = &button.image {
                if show_play_icon {
                    write_overlayed(idx, &img, play_img, deck)?;
                } else {
                    write_image(&idx, deck, img.clone())?;
                }
            } else {
                write_image(&idx, deck, play_img.clone())?;
            }
        } else {
            if let Some(path) = &button.config.sound {
                let file = BufReader::new(File::open(path)?);
                let source = Decoder::new(file)?;
                sink.stop();
                debug!("Playing audio file {:?}", path);
                sink.append(source);
                button.playing = true;
                if let Some(img) = &button.image {
                    write_overlayed(idx, &img, stop_img, deck)?;
                } else {
                    write_image(&idx, deck, stop_img.clone())?;
                }
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
            if let Err(err) = mqtt.publish(message) {
                error!(
                    "Unable to send message with payload of {} to topic {}: {}",
                    payload, topic, err
                );
            };
        }
    } else {
        debug!("Buttons released");
    }
    Ok(())
}
