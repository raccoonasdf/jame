use chrono::{DateTime, NaiveDateTime, Utc};
use rand::{random, seq::SliceRandom};
use serenity::{
    async_trait,
    builder::CreateMessage,
    http::client::Http,
    model::{
        channel::{Message, PermissionOverwrite, PermissionOverwriteType},
        event::TypingStartEvent,
        gateway::Ready,
        id::{ChannelId, RoleId},
        permissions::Permissions,
    },
    prelude::*,
};
use std::{
    env,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::time::{sleep, Duration};

const GOOD_NAMES: &[&str] = &["jame"];
const BAD_NAMES: &[&str] = &["james"];

const RESPONSE_GREETINGS: &[&str] = &["hey", "hi", "hihi", "hello"];
const RESPONSE_NAMES: &[&str] = &["im jame", "jame", "hi im jame"];
const RESPONSE_PREFIXES: &[&str] = &["its me ", "", ""];

const SWAP_TIME: Duration = Duration::from_secs(13 * 59); // 59-second minutes for consistency

const WORDS_A: ChannelId = ChannelId(808615833026822144);
const WORDS_B: ChannelId = ChannelId(808776792005148712);
const EVERYONE: RoleId = RoleId(808615832568856617);

#[derive(Copy, Clone, Debug)]
enum Words {
    A,
    B,
}

impl Words {
    fn channel_id(&self) -> ChannelId {
        use Words::*;
        match self {
            A => WORDS_A,
            B => WORDS_B,
        }
    }

    fn swapped(&self) -> Words {
        use Words::*;
        match self {
            A => B,
            B => A,
        }
    }
}

struct LastWordsInteraction;

impl TypeMapKey for LastWordsInteraction {
    type Value = DateTime<Utc>;
}

struct CurrentWords;

impl TypeMapKey for CurrentWords {
    type Value = Words;
}

struct IsFirstReady;

impl TypeMapKey for IsFirstReady {
    type Value = AtomicBool;
}

struct Handler;

impl Handler {
    async fn send_typed_message<'a, F>(
        &self,
        http: &Arc<Http>,
        channel_id: ChannelId,
        min_length: f64,
        random_added_length: f64,
        f: F,
    ) where
        for<'b> F: FnOnce(&'b mut CreateMessage<'a>) -> &'b mut CreateMessage<'a>,
    {
        let s = sleep(Duration::from_millis(
            (random::<f64>() * random_added_length + min_length) as u64,
        ));

        if let Ok(typing) = channel_id.start_typing(http) {
            s.await;
            typing.stop();
        } else {
            println!("Failed to signal typing");
            s.await;
        }

        if let Err(_) = channel_id.send_message(http, f).await {
            println!("Failed to send message")
        }
    }

    async fn last_words_update(
        &self,
        ctx: &Context,
        channel_id: ChannelId,
        timestamp: DateTime<Utc>,
    ) {
        let mut data = ctx.data.write().await;
        let current_words = data.get::<CurrentWords>().unwrap();
        let last_words_interaction = data.get::<LastWordsInteraction>().unwrap();

        if current_words.channel_id() == channel_id && last_words_interaction < &timestamp {
            data.insert::<LastWordsInteraction>(timestamp);
        }
    }

    async fn switch(&self, ctx: &Context) {
        let current_words = {
            let data = ctx.data.read().await;

            *data.get::<CurrentWords>().unwrap()
        };

        if let Err(_) = current_words
            .channel_id()
            .create_permission(
                &ctx.http,
                &PermissionOverwrite {
                    allow: Permissions::empty(),
                    deny: Permissions::READ_MESSAGES,
                    kind: PermissionOverwriteType::Role(EVERYONE),
                },
            )
            .await
        {
            println!("Failed to change channel permissions");
        }

        let new_words = current_words.swapped();
        if let Err(_) = new_words
            .channel_id()
            .delete_permission(&ctx.http, PermissionOverwriteType::Role(EVERYONE))
            .await
        {
            println!("Failed to change channel permissions");
        }

        {
            let mut data = ctx.data.write().await;
            data.insert::<CurrentWords>(new_words);
            data.insert::<LastWordsInteraction>(Utc::now());
        }

        println!("swapped to: {:?}", new_words);
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.id != ctx.cache.current_user_id().await {
            self.last_words_update(&ctx, msg.channel_id, msg.timestamp)
                .await;

            if msg.content == "!swish" {
                self.switch(&ctx).await;
                return;
            }

            let msg_chance = random::<f64>();

            let response = {
                let lower_content = msg.content.to_lowercase();
                let split_content: Vec<&str> = lower_content
                    .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
                    .collect();

                fn any_matches(a: &[&str], b: &[&str]) -> bool {
                    a.into_iter()
                        .any(|name| b.into_iter().any(|word| word == name))
                }

                let any_bad = any_matches(BAD_NAMES, split_content.as_slice());
                let any_good = any_matches(GOOD_NAMES, split_content.as_slice());

                if any_bad && msg_chance <= 0.75 {
                    Some("no".to_string())
                } else if any_good && msg_chance <= 0.9 || msg_chance <= 0.005 {
                    let rng = &mut rand::thread_rng();
                    let greeting = *RESPONSE_GREETINGS.choose(rng).unwrap();
                    let name = *RESPONSE_NAMES.choose(rng).unwrap();
                    let prefix = *RESPONSE_PREFIXES.choose(rng).unwrap();

                    if random::<f64>() <= 0.75 {
                        Some(format!("{} {}{}", greeting, prefix, name))
                    } else {
                        Some(format!("{}{} {}", prefix, name, greeting))
                    }
                } else {
                    None
                }
            };

            if let Some(response) = response {
                println!("typing: {}", response);

                self.send_typed_message(&ctx.http, msg.channel_id, 750.0, 1250.0, |m| {
                    m.content(response);
                    m
                })
                .await;
            }
        }
    }

    async fn typing_start(&self, ctx: Context, typing: TypingStartEvent) {
        if typing.user_id != ctx.cache.current_user_id().await {
            let timestamp = DateTime::<Utc>::from_utc(
                NaiveDateTime::from_timestamp(typing.timestamp as i64, 0),
                Utc,
            );

            self.last_words_update(&ctx, typing.channel_id, timestamp)
                .await;
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        println!("ready as {}", ready.user.name);

        let is_first_ready = {
            let data = ctx.data.read().await;

            let is_first_ready = data.get::<IsFirstReady>().unwrap();

            is_first_ready.swap(false, Ordering::Relaxed) // swap returns the pre-swap value
        };

        if is_first_ready {
            println!("started loop");

            loop {
                let duration = {
                    let data = ctx.data.read().await;

                    let last_interaction = *data.get::<LastWordsInteraction>().unwrap();
                    let swap_time = chrono::Duration::from_std(SWAP_TIME).unwrap();

                    last_interaction + swap_time - Utc::now()
                };

                match duration.to_std() {
                    Ok(t) => {
                        sleep(t).await;
                    }
                    Err(_) => {
                        self.switch(&ctx).await;
                    }
                }
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let mut client = Client::builder(env::var("TOKEN").expect("no TOKEN in env"))
        .event_handler(Handler)
        .await
        .expect("client build error");

    {
        let mut data = client.data.write().await;
        data.insert::<LastWordsInteraction>(Utc::now());
        data.insert::<CurrentWords>(Words::A);
        data.insert::<IsFirstReady>(AtomicBool::new(true));
    }

    if let Err(e) = client.start().await {
        println!("client run error: {:?}", e);
    }
}
