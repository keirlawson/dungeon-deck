use paho_mqtt as mqtt;

#[tokio::main]
async fn main() {
    let broker_url = format!("mqtt://{}", env!("MQTT_HOST"));
    let broker_user = env!("MQTT_USER");
    let broker_pass = env!("MQTT_PASS");

    let con_opts = mqtt::ConnectOptionsBuilder::new()
        .user_name(broker_user)
        .password(broker_pass)
        .finalize();

    let client = mqtt::AsyncClient::new(broker_url).unwrap();

    client.connect(Some(con_opts)).await.unwrap();

    println!("Hello, world!");
}
