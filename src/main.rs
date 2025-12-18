use log::{info, debug, warn, error, LevelFilter, Metadata, Log, Record};
use clap::Parser;
use std::sync::Mutex;
use std::process::Stdio;
use once_cell::sync::Lazy;

struct Logger;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    mqtt_ip: Option<String>,

    #[arg(short, long)]
    agent_socket_path: Option<String>,

    #[arg(short, long)]
    bind_id: Option<u32>,

    #[arg(short, long)]
    log_level: Option<String>,
}

#[allow(dead_code)]
struct SendingTopicCommand {
    id: u64,
    to: u64,
    from: u64
}

impl Log for Logger {
    fn enabled(&self, _meta: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        eprintln!("{}: {}", record.level(), record.args());
    }

    fn flush(&self) {}
}

use paho_mqtt as mqtt;
use tokio::{
    sync::mpsc,
    time::{sleep, Duration},
    process::Command,
    io::{AsyncBufReadExt, BufReader}
};

use tokio_seqpacket::UnixSeqpacket;
use tokio_stream::StreamExt;
use serde_json::Value;

const TOPIC_COMMAND: &str = "miio/command";
const TOPIC_COMMAND_ACK: &str = "miio/command_ack";
const TOPIC_RESPONSE: &str = "openmiio/report";
static SENDING_TOPIC_COMMAND: Lazy<Mutex<SendingTopicCommand>> = Lazy::new(|| {
    Mutex::new(SendingTopicCommand {
        id: 0,
        to: 0,
        from: 0
    })
});


async fn mqtt_reconnect(client: &mqtt::AsyncClient) {
    loop {
        if client.reconnect().await.is_ok() {
            if mqtt_subscribe(client).await {
                warn!("Successfully reconnected");
                return;
            }
        }
        sleep(Duration::from_millis(500)).await;
    }
}

async fn mqtt_subscribe(client: &mqtt::AsyncClient) -> bool {
    let subscribe_result = client.subscribe(TOPIC_COMMAND, 0).await.and_then(|rsp| {
        rsp.subscribe_response()
            .ok_or(mqtt::Error::General("Bad response"))
    });
    if let Err(err) = subscribe_result {
        let _ = client.disconnect(None).await;
        error!("Error subscribing to topics: {:?}", err);
        return false;
    }
    true
}

async fn mqtt_manager(
    mut mqtt_client: mqtt::AsyncClient,
    command_tx: mpsc::Sender<String>,
) {
    let conn_opts = mqtt::ConnectOptionsBuilder::new()
        .keep_alive_interval(Duration::from_secs(20))
        .clean_session(true)
        .finalize();

    // Make the connection to the broker
    loop {
        info!(
            "Connecting to the MQTT broker at '{}'...",
            mqtt_client.server_uri()
        );
        match mqtt_client.connect(conn_opts.clone()).await {
            Ok(response) => {
                if let Some(response) = response.connect_response() {
                    info!(
                        "Connected to: '{}' with MQTT version {}",
                        response.server_uri, response.mqtt_version
                    );

                    mqtt_subscribe(&mqtt_client).await;
                    break;
                }
            }
            Err(e) => {
                error!("Error connecting to the MQTT broker: {:?}", e);
                sleep(Duration::from_millis(500)).await;
            }
        }
    }

    // Outer loop to recreate stream if it closes
    loop {
        let mut stream = mqtt_client.get_stream(25);

        while let Some(msg) = stream.next().await {
            match msg {
                Some(msg) => {
                    if msg.topic() == TOPIC_COMMAND {
                        debug!("get command '{}'", msg);
                        let payload = msg.payload_str().to_string();
                        if let Err(e) = command_tx.send(payload.clone()).await {
                            error!("Error sending command to agent task: {:?}", e);
                        }
                        match serde_json::from_str::<Value>(&payload) {
                            Ok(json_msg) => {
                                let mut sending_command = SENDING_TOPIC_COMMAND.lock().unwrap();
                                if let Some(id) = json_msg.get("id").and_then(|v| v.as_u64()) {
                                    sending_command.id = id;
                                }
                                if let Some(to) = json_msg.get("_to").and_then(|v| v.as_u64()) {
                                    sending_command.to = to;
                                }
                                if let Some(from) = json_msg.get("_from").and_then(|v| v.as_u64()) {
                                    sending_command.from = from;
                                }
                                debug!("id: {}", sending_command.id);
                                debug!("to: {}", sending_command.to);
                                debug!("from: {}", sending_command.from);
                            }
                            Err(e) => {
                                error!("Failed to parse JSON from MQTT: {:?}", e);
                                continue;
                            }
                        }
                    }
                }
                None => {
                    warn!("MQTT Connection lost. Reconnecting...");
                    mqtt_reconnect(&mqtt_client).await;
                }
            }
        }
        info!("MQTT stream ended. Re-acquiring stream...");
        sleep(Duration::from_millis(1000)).await;
    }
}

async fn ha_driven_reader(
    mqtt_client: mqtt::AsyncClient
) {
    let _ = Command::new("killall").arg("-9").arg("ha_driven").spawn();
    sleep(Duration::from_millis(500)).await;

    let mut command = Command::new("ha_driven");
    command.stdout(Stdio::piped());
    info!("Preparing to read logs from ha_driven...");

    let mut child = command.spawn().expect("Failed to spawn child process");

    let stdout = child.stdout.take().expect("Failed to open stdout");

    let mut reader = BufReader::new(stdout).lines();

    while let Ok(Some(line)) = reader.next_line().await {
        if line.contains("onReceiveMessage") && line.contains("method") && line.contains("res/report") {
        //if line.contains("onReceiveMessage") && line.contains("method") && line.contains("res/report") && line.contains("res_list") {
            let s = line.split(">>").nth(1).unwrap();
            let s2 = s.trim().split(" ").nth(0).unwrap();
            println!("Captured line1: {}", s2);

            let _ = mqtt_client
                .publish(mqtt::Message::new(TOPIC_RESPONSE, s2.to_string().as_bytes(), 0));
            continue;
        }
    }
}

async fn agent_manager(
    agent_socket_path: &str,
    mqtt_client: mqtt::AsyncClient,
    mut command_rx: mpsc::Receiver<String>,
    bind_id: u32,
) {
    let mut buf = [0; 4096];

    loop {
        info!("Connecting to the miio agent socket at '{}'...", agent_socket_path);

        let agent_socket = loop {
            if let Ok(socket) = UnixSeqpacket::connect(agent_socket_path).await {
                info!("Successfully connected to miio agent socket with {}", bind_id);
                // Send initialization messages
                let _ = socket.send(format!(r#"{{"address":{},"method":"bind"}}"#, bind_id).as_bytes()).await;
                for msg in [
                    r#"{"key":"auto.report","method":"register"}"#,
                    r#"{"key":"auto.forward","method":"register"}"#,
                    r#"{"key":"lanbox.event","method":"register"}"#,
                    r#"{"key":"auto.ifttt","method":"register"}"#,
                    r#"{"key":"auto.cross.ifttt","method":"register"}"#,
                    r#"{"key":"matter.control","method":"register"}"#,
                    r#"{"key":"matter.event","method":"register"}"#,
                    r#"{"key":"mtbr.control","method":"register"}"#,
                ] {
                    let _ = socket.send(msg.as_bytes()).await;
                }
                break socket;
            }
            sleep(Duration::from_millis(500)).await;
        };

        loop {
            tokio::select! {
                // Receive commands from MQTT task
                cmd = command_rx.recv() => {
                    match cmd {
                        Some(payload) => {
                            if let Err(e) = agent_socket.send(payload.as_bytes()).await {
                                error!("Error sending to agent socket: {:?}. Reconnecting...", e);
                                break;
                            }
                        },
                        None => return, // Channel closed, exit application
                    }
                }
                // Receive data from Agent Socket
                res = agent_socket.recv(&mut buf) => {
                    match res {
                        Ok(n) if n > 0 => {
                            let mut topic: &str = TOPIC_RESPONSE;
                            match serde_json::from_slice::<Value>(&buf[..n]) {
                                Ok(msg) => {
                                    debug!("reading length: '{}' msg: '{:?}'", n, msg);

                                    // Check if this message correlates to the last command sent
                                    if let Some(recv_id) = msg.get("id").and_then(|v| v.as_u64()) {
                                        let sending_command = SENDING_TOPIC_COMMAND.lock().unwrap();
                                        if sending_command.id == recv_id {
                                            topic = TOPIC_COMMAND_ACK;
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to parse JSON from agent: {:?}", e);
                                    continue;
                                }
                            }

                            let _ = mqtt_client
                                .publish(mqtt::Message::new(topic, &buf[..n], 0))
                                .await;
                        }
                        Ok(_) => {
                            warn!("Agent socket closed (EOF). Reconnecting...");
                            break;
                        }
                        Err(e) => {
                            error!("Error reading from agent socket: {:?}. Reconnecting...", e);
                            break;
                        }
                    }
                }
            }
        }
        sleep(Duration::from_millis(500)).await;
    }
}

fn init_log(log_level: LevelFilter) {
    static LOGGER: Logger = Logger;
    log::set_max_level(log_level);
    log::set_logger(&LOGGER).unwrap();
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = Cli::parse();

    let level = match cli.log_level {
        Some(level) => match level.as_str() {
            "error" => LevelFilter::Error,
            "warn" => LevelFilter::Warn,
            "info" => LevelFilter::Info,
            "debug" => LevelFilter::Debug,
            "trace" => LevelFilter::Trace,
            _ => LevelFilter::Info,
        },
        None => LevelFilter::Info,
    };

    init_log(level);

    let mqtt_host = match cli.mqtt_ip {
        Some(ip) => format!("mqtt://{}:1883", ip),
        None => "mqtt://localhost:1883".to_string(),
    };

    let create_opts = mqtt::CreateOptionsBuilder::new()
        .server_uri(mqtt_host)
        .client_id("agent2mqtt")
        .finalize();
    let mqtt_client = mqtt::AsyncClient::new(create_opts).unwrap_or_else(|e| {
        panic!("Error creating the MQTT client: {:?}", e);
    });

    let bind_id = match cli.bind_id {
        Some(id) => id,
        None => 0,
    };

    let agent_socket_path = match cli.agent_socket_path {
        Some(path) => path,
        None => "/tmp/miio_agent.socket".to_string(),
    };

    let (tx, rx) = mpsc::channel::<String>(32);

    tokio::spawn(mqtt_manager(
        mqtt_client.clone(),
        tx,
    ));

    tokio::spawn(ha_driven_reader(
        mqtt_client.clone(),
    ));

    agent_manager(&agent_socket_path, mqtt_client, rx, bind_id).await;
}
