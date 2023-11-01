# Dungeon Deck
Turn your Stream Deck Mini into a headless sound/automation board.
It supports displaying images on the respective buttons, playing sounds
as well as interacting with home automations via MQTT.

## Usage

```sh
dungeon-deck
```

### Run at startup

In order to run this application headless on the likes of a 
[Pi Zero 2 W](https://www.raspberrypi.com/products/raspberry-pi-zero-2-w/)
SystemD can be utilised.  First in order to ensure that the PuseAudio service
is running even without a user logging in we need to enable "lingering" for 
that user, which should cause associated SystemD services to start running at
boot:

```sh
loginctl enable-linger $USER
```

We can then use our own service to control starting up, example of this approach,
along with a service to connect to a bluetooth speaker on boot can be found in
the [examples directory](./examples/).  These service files should be placed in 
`~/.config/systemd/user/` and enabled as so:

```sh
systemctl --user enable btconnect.service
systemctl --user enable dungeondeck.service
``` 

## Configuration
Dungeon Deck reads in configuration for a `dungeon.toml` file located in 
its working directory. Example configuration:

```toml
[mqtt]
host = "mymqtthost" 
user = "someusername" 
pass = "somepassword"

[buttons]

[buttons.top.left]
image = "./swords.png"
sound = "./epicbattle.mp3"

[buttons.bottom.right]
image = "./lightbulb.png"
topic = "some/topic"
payload = "pressed"
```

## Development

### cross-building for ARM

We can cross-build for ARM using [cross](https://github.com/cross-rs/cross):

```sh
cross build --target aarch64-unknown-linux-gnu
```

### Releasing

Releases are automatically built and published for any tag with the form `v*.*.*`
