# Dungeon Deck
Turn your Stream Deck Mini into a headless sound/automation board.
It supports displaying images on the respective buttons, playing sounds
as well as interacting with home automations via MQTT.

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
