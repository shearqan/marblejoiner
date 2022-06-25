use clap::Parser;
use std::collections::HashMap;
use std::iter;
use std::ops::Add;
use std::thread;
use std::time::{Duration, Instant};
use twitch_irc::{
    login::{CredentialsPair, StaticLoginCredentials},
    message::{PrivmsgMessage, ServerMessage},
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
        help= "The message the app joins the race for you with"
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
struct MainState {
    buffer_size: usize,
    treshhold: usize,
    delay: Duration,
    wait: Duration,
    play_message: String,
    login: String,
    oauth: String,
    channel_states: HashMap<String, ChannelMarbleState>,
}

#[derive(Debug)]
struct ChannelMarbleState {
    buffer: Vec<bool>,
    current_position: usize,
    next_play: Instant,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    let marble_states: HashMap<String, ChannelMarbleState> = args
        .channels
        .into_iter()
        .map(|channel| (channel, ChannelMarbleState::new(args.buffer_size)))
        .collect();
    let mut main_state = MainState {
        buffer_size: args.buffer_size,
        treshhold: args.treshhold,
        delay: Duration::from_secs(args.delay),
        wait: Duration::from_secs(args.wait),
        play_message: args.play_message,
        login: args.login,
        oauth: args.oauth.replacen("oauth:", "", 1),
        channel_states: marble_states,
    };

    let credentials = StaticLoginCredentials {
        credentials: CredentialsPair {
            login: main_state.login.to_owned(),
            token: Some(main_state.oauth.to_owned()),
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
        for channel in main_state.channel_states.keys() {
            client.join(channel.to_string()).unwrap();
        }

        while let Some(message) = incoming_messages.recv().await {
            println!("Received message: {:?}", message);
            println!();
            let _ = handle_new_message(&message, &mut main_state, &client).await;
        }
    });

    join_handle.await.unwrap();

    Ok(())
}

impl MainState {
    pub async fn process_message(
        self: &mut MainState,
        message: &PrivmsgMessage,
        client: &TwitchIRCClient<SecureTCPTransport, StaticLoginCredentials>,
    ) -> anyhow::Result<()> {
        let channel_state = self.channel_states.get_mut(&message.channel_login).unwrap();
        channel_state.current_position = (channel_state.current_position + 1) % self.buffer_size;
        channel_state.buffer[channel_state.current_position] =
            message.message_text.starts_with("!play");
        if channel_state.is_time_to_play(self.delay)
            && channel_state.is_treshhold_reached(self.treshhold)
        {
            channel_state.next_play = Instant::now().add(self.delay);
            thread::sleep(self.wait);
            client
                .say(message.channel_login.to_owned(), self.play_message.to_owned())
                .await?;
        }
        Ok(())
    }
}

impl ChannelMarbleState {
    fn new(buffer_size: usize) -> ChannelMarbleState {
        ChannelMarbleState {
            buffer: iter::repeat(false).take(buffer_size).collect(),
            current_position: 0,
            next_play: Instant::now(),
        }
    }

    fn is_time_to_play(self: &ChannelMarbleState, delay: Duration) -> bool {
        self.next_play < Instant::now()
    }

    fn is_treshhold_reached(self: &ChannelMarbleState, treshhold: usize) -> bool {
        self.buffer.iter().filter(|x| **x).count() >= treshhold
    }
}

async fn handle_new_message(
    server_message: &ServerMessage,
    main_state: &mut MainState,
    client: &TwitchIRCClient<SecureTCPTransport, StaticLoginCredentials>,
) -> anyhow::Result<()> {
    match server_message {
        ServerMessage::Privmsg(message) => {
            main_state.process_message(message, client).await?;
        }
        _ => {}
    }
    Ok(())
}
