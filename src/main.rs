use clap::Parser;
use std::collections::HashMap;
use std::iter;
use std::ops::Add;
use std::thread;
use std::time::{Duration, Instant};
use twitch_irc::{
    login::{CredentialsPair, StaticLoginCredentials},
    message::ServerMessage,
    ClientConfig, SecureTCPTransport, TwitchIRCClient,
};

#[derive(Parser, Default, Debug)]
#[clap(
    author = "shearqan",
    about = "An app that joins marbles on stream races for you"
)]
struct Cli {
    #[clap(
        short,
        long,
        default_value_t = 10,
        value_parser,
        help = "How many last messages are considered"
    )]
    buffer_size: usize,

    #[clap(
        short,
        long,
        default_value_t = 5,
        value_parser,
        help = "How many of the buffered messages have to start with !play in order to trigger"
    )]
    treshhold: usize,
    #[clap(
        short,
        long,
        default_value_t = 120,
        value_parser,
        help = "Delay in seconds on minimum time between two plays from this app"
    )]
    delay: u64,

    #[clap(
        short,
        long,
        default_value_t = 5,
        value_parser,
        help = "Time to wait in seconds before posting the !play due to the idiotic combination of the game not letting you join before the cutscene on some map starts and fucking idiots joining during the loading screen already"
    )]
    wait: u64,

    #[clap(
        short,
        long,
        default_value = "!play >:(",
        value_parser,
        help = "The message the app joins the race for you with"
    )]
    play_message: String,

    #[clap(forbid_empty_values = true, help = "Your twitch login name")]
    login: String,

    #[clap(
        forbid_empty_values = true,
        help = "Your oauth authorization token according to https://dev.twitch.tv/docs/authentication/getting-tokens-oauth, if you have no idea what this means, use this site to get one: https://twitchapps.com/tmi/"
    )]
    oauth: String,

    #[clap(
        last = true,
        forbid_empty_values = true,
        multiple_values = true,
        value_delimiter = ' ',
        help = "Channels you want to play in, separated with a space"
    )]
    channels: Vec<String>,
}

#[derive(Debug)]
struct AppParams {
    buffer_size: usize,
    treshhold: usize,
    delay: Duration,
    wait: Duration,
    play_message: String,
    login: String,
    oauth: String,
}

#[derive(Debug)]
struct ChannelMarbleState {
    login: String,
    buffer: Vec<bool>,
    current_position: usize,
    next_play: Instant,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    let mut marble_states: HashMap<String, ChannelMarbleState> = args
        .channels
        .into_iter()
        .map(|channel| (channel.to_owned(), ChannelMarbleState::new(channel, args.buffer_size)))
        .collect();
    let app_params = AppParams {
        buffer_size: args.buffer_size,
        treshhold: args.treshhold,
        delay: Duration::from_secs(args.delay),
        wait: Duration::from_secs(args.wait),
        play_message: args.play_message,
        login: args.login,
        oauth: args.oauth.replacen("oauth:", "", 1),
    };

    let credentials = StaticLoginCredentials {
        credentials: CredentialsPair {
            login: app_params.login.to_owned(),
            token: Some(app_params.oauth.to_owned()),
        },
    };
    let config = ClientConfig {
        login_credentials: credentials,
        ..ClientConfig::default()
    };
    let (mut incoming_messages, client) =
        TwitchIRCClient::<SecureTCPTransport, StaticLoginCredentials>::new(config);
    let client = Box::new(client);

    let join_handle = tokio::spawn(async move {
        for channel in marble_states.keys() {
            client.join(channel.to_string()).unwrap();
        }

        while let Some(message) = incoming_messages.recv().await {
            println!("Received message: {:?}", message);
            println!();
            let _ = process_message(&app_params, &mut marble_states, &message, &client).await;
        }
    });

    join_handle.await.unwrap();

    Ok(())
}

async fn process_message(
    app_params: &AppParams,
    marble_states: &mut HashMap<String, ChannelMarbleState>,
    server_message: &ServerMessage,
    client: &TwitchIRCClient<SecureTCPTransport, StaticLoginCredentials>,
) -> anyhow::Result<()> {
    match server_message {
        ServerMessage::Privmsg(message) => {
            marble_states
                .get_mut(&message.channel_login)
                .unwrap()
                .process_message(app_params, &message.message_text, client)
                .await?;
        }
        _ => {}
    }

    Ok(())
}

impl ChannelMarbleState {
    fn new(login: String, buffer_size: usize) -> ChannelMarbleState {
        ChannelMarbleState {
            login,
            buffer: iter::repeat(false).take(buffer_size).collect(),
            current_position: 0,
            next_play: Instant::now(),
        }
    }

    async fn process_message(
        self: &mut ChannelMarbleState,
        app_params: &AppParams,
        message: &str,
        client: &TwitchIRCClient<SecureTCPTransport, StaticLoginCredentials>,
    ) -> anyhow::Result<()> {
        self.current_position = (self.current_position + 1) % app_params.buffer_size;
        self.buffer[self.current_position] = message.starts_with("!play");
        if self.is_time_to_play() && self.is_treshhold_reached(app_params) {
            self.next_play = Instant::now().add(app_params.delay);
            self.clear_buffer();
            thread::sleep(app_params.wait);
            client.say(self.login.to_owned(), app_params.play_message.to_owned()).await?;
        }

        Ok(())
    }

    fn is_time_to_play(self: &ChannelMarbleState) -> bool {
        self.next_play < Instant::now()
    }

    fn is_treshhold_reached(self: &ChannelMarbleState, app_params: &AppParams) -> bool {
        self.buffer.iter().filter(|x| **x).count() >= app_params.treshhold
    }

    fn clear_buffer(self: &mut ChannelMarbleState) {
        for i in 0..self.buffer.len() {
            self.buffer[i] = false;
        }
    }
}
