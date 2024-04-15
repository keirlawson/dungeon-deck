use ab_glyph::FontRef;
use ab_glyph::PxScale;
use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use env_logger::Env;
use image::imageops;
use image::imageops::FilterType;
use image::io::Reader;
use image::DynamicImage;
use image::ImageBuffer;
use image::Rgb;
use log::debug;
use log::error;
use log::info;
use mqtt::Client;
use paho_mqtt as mqtt;
use rodio::{Decoder, OutputStream, Sink};
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::iter;
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

const FONT_DATA: &[u8] = include_bytes!("../unifont.otf");
const BLACK: Rgb<u8> = Rgb([0, 0, 0]);
const WHITE: Rgb<u8> = Rgb([255, 255, 255]);

struct Button {
    config: ButtonConfig,
    playing: bool,
    image: Option<DynamicImage>,
}

impl Default for Button {
    fn default() -> Self {
        Button {
            config: ButtonConfig {
                text: None,
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
    text: Option<String>,
    image: Option<PathBuf>,
    sound: Option<PathBuf>,
    topic: Option<String>,
    payload: Option<String>,
}

#[derive(Deserialize)]
struct Row {
    one: Option<ButtonConfig>,
    two: Option<ButtonConfig>,
    three: Option<ButtonConfig>,
    four: Option<ButtonConfig>,
    five: Option<ButtonConfig>,
    six: Option<ButtonConfig>,
    seven: Option<ButtonConfig>,
    eight: Option<ButtonConfig>,
}

impl Row {
    fn list(self, limit: usize) -> Vec<Option<ButtonConfig>> {
        [
            self.one, self.two, self.three, self.four, self.five, self.six, self.seven, self.eight,
        ]
        .into_iter()
        .take(limit)
        .collect()
    }
}

#[derive(Deserialize)]
struct Buttons {
    first: Option<Row>,
    second: Option<Row>,
    third: Option<Row>,
    fourth: Option<Row>,
}

impl Buttons {
    fn list(self, device: Device) -> Vec<Option<ButtonConfig>> {
        let it = [self.first, self.second, self.third, self.fourth].into_iter();
        let it = it.take(device.rows());
        it.flat_map(|row| match row {
            Some(r) => r.list(device.columns()),
            None => iter::repeat(None).take(device.columns()).collect(),
        })
        .collect()
    }
}

#[derive(Deserialize)]
enum Device {
    Original,
    OriginalV2,
    Mk2,
    Mini,
    RevisedMini,
    XL,
}

impl Device {
    fn rows(&self) -> usize {
        match self {
            Device::Mk2 | Device::Original | Device::OriginalV2 => 3,
            Device::RevisedMini | Device::Mini => 2,
            Device::XL => 4,
        }
    }

    fn columns(&self) -> usize {
        match self {
            Device::Mk2 | Device::Original | Device::OriginalV2 => 5,
            Device::RevisedMini | Device::Mini => 3,
            Device::XL => 8,
        }
    }
}

#[derive(Deserialize)]
struct Config {
    device: Device,
    mqtt: Option<BrokerConfig>,
    buttons: Buttons,
    #[serde(default)]
    playicon: bool,
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

fn display_buttons(
    state: &HashMap<usize, Button>,
    deck: &mut StreamDeck,
    play_img: &DynamicImage,
    show_play_icon: bool,
) -> Result<()> {
    //FIXME still render initial play button when image not set
    state.iter().try_for_each(|(index, button)| {
        if let Some(img) = &button.image {
            let mut img = img.clone();
            if button.config.sound.is_some() && show_play_icon {
                imageops::overlay(&mut img, play_img, 0, 0);
            }
            write_image(index, deck, img)?;
        }

        Ok(())
    })
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

    let config: Config = toml::from_str(&config_contents)?;

    let font = FontRef::try_from_slice(FONT_DATA)?;

    let broker_client = config.mqtt.map(connect_mqtt).transpose()?;

    const ELGATO_VID: u16 = 0x0fd9;
    let pid = match config.device {
        Device::Mk2 => pids::MK2,
        Device::RevisedMini => pids::REVISED_MINI,
        Device::Original => pids::ORIGINAL,
        Device::OriginalV2 => pids::ORIGINAL_V2,
        Device::Mini => pids::MINI,
        Device::XL => pids::XL,
    };
    let mut deck = StreamDeck::connect(ELGATO_VID, pid, None)?;
    info!("Connected to Stream Deck");

    let (width, height) = deck.kind().image_size();
    let play_img = image::load_from_memory(PLAY_IMG)?;
    let stop_img = image::load_from_memory(STOP_IMG)?;
    let buttons = config.buttons.list(config.device);
    let mut button_state = build_state(buttons, width, height, font)?;
    display_buttons(&button_state, &mut deck, &play_img, config.playicon)?;

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
        if kill.load(Ordering::Relaxed) {
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
    font: FontRef,
) -> Result<HashMap<usize, Button>> {
    buttons
        .into_iter()
        .map(move |conf| {
            let button = if let Some(conf) = conf {
                let image = prepare_image(&conf, width, height, &font)?;

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

fn prepare_image(
    conf: &ButtonConfig,
    width: usize,
    height: usize,
    font: &FontRef,
) -> Result<Option<DynamicImage>, anyhow::Error> {
    if let Some(text) = &conf.text {
        let mut image = ImageBuffer::from_pixel(width as u32, height as u32, BLACK);
        let line_height = 1.1;
        let scale = PxScale { x: 15.0, y: 15.0 };
        let mut y = 0;
        text.to_string().split('\n').for_each(|txt| {
            imageproc::drawing::draw_text_mut(&mut image, WHITE, 0, y, scale, &font, txt);
            y += (scale.y * line_height).round() as i32;
        });
        Ok(Some(DynamicImage::ImageRgb8(image)))
    } else if let Some(path) = &conf.image {
        let image = Reader::open(path)
            .with_context(|| format!("Unable to open path {}", path.display()))?
            .decode()?;
        let image = image.resize(width as u32, height as u32, FilterType::Gaussian);
        Ok(Some(image))
    } else {
        Ok(None)
    }
}

fn pressed_idx(states: &[u8]) -> Option<usize> {
    states
        .iter()
        .enumerate()
        .find(|(_, state)| state == &&1)
        .map(|(i, _)| i)
}

#[allow(clippy::too_many_arguments)]
fn handle_press(
    sink: &Sink,
    buttons: &mut HashMap<usize, Button>,
    pressed: &[u8],
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
                    write_overlayed(idx, img, play_img, deck)?;
                } else {
                    write_image(&idx, deck, img.clone())?;
                }
            } else {
                write_image(&idx, deck, play_img.clone())?;
            }
        } else if let Some(path) = &button.config.sound {
            let file = BufReader::new(File::open(path)?);
            let source = Decoder::new(file)?;
            sink.stop();
            debug!("Playing audio file {:?}", path);
            sink.append(source);
            button.playing = true;
            if let Some(img) = &button.image {
                write_overlayed(idx, img, stop_img, deck)?;
            } else {
                write_image(&idx, deck, stop_img.clone())?;
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
