use actix::StreamHandler;
use actix_web::web;
use actix_web_actors::ws;
use actix::prelude::*;
use openai_dive::v1::{api::Client, resources::chat_completion::{ChatCompletionParameters, ChatMessage}};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;
use crate::{
    data::models::{self, Pool},
    operators::message_operator::{
        get_messages_for_topic_query, user_owns_topic_query, ChatCompletionDTO
    },
};

#[derive(Serialize, Deserialize, Debug)]
struct MessageDTO {
    command: String,
    previous_messages: Option<Vec<models::Message>>,
    topic_id: Option<uuid::Uuid>,
}

#[derive(Serialize, Deserialize, Debug)]
enum Command {
    Ping,
    Prompt(Vec<models::Message>),
    RegenerateMessage,
    ChangeTopic(uuid::Uuid),
    Stop,
    InvalidMessage(String),
}

#[derive(Serialize, Deserialize, Debug)]
enum Response {
    Messages(Vec<models::Message>),
    ChatMessage(String),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct CompletionWebSeocket {
    pub user_id: uuid::Uuid,
    pub topic_id: Option<uuid::Uuid>,
    pub last_pong: chrono::DateTime<chrono::Utc>,
    pub pool: web::Data<Pool>,
    pub spawn_handle: Option<actix::SpawnHandle>,
}

impl From<ws::Message> for Command {
    fn from(message: ws::Message) -> Self {
        match message {
            ws::Message::Ping(_msg) => {
                Command::Ping
            }
            ws::Message::Text(text) => {
                let parsed_message: Result<MessageDTO, serde_json::Error> = serde_json::from_str(&text);
                if parsed_message.is_err() {
                    return Command::InvalidMessage("Invalid message".to_string());
                }
                let message = parsed_message.unwrap();
                match (&message, message.command.as_str()) {
                    (_, "ping") => Command::Ping,
                    (msg, "prompt") if msg.previous_messages.is_some() => {
                        Command::Prompt(message.previous_messages.unwrap())
                    },
                    (_, "regenerateMessage") => Command::RegenerateMessage,
                    (msg, "changeTopic") if msg.topic_id.is_some() => {
                        Command::ChangeTopic(message.topic_id.unwrap())
                    },
                    (_, "stop") => Command::Stop,
                    (_, _) => Command::InvalidMessage("Missing properties".to_string()),
                }
            }
            ws::Message::Binary(_) =>Command::InvalidMessage("Binary not a valid operation".to_string()),
            ws::Message::Close(_) => Command::InvalidMessage("Close not a operation".to_string()),
            ws::Message::Continuation(_) => Command::InvalidMessage("Continuation not a operation".to_string()),
            ws::Message::Nop => Command::InvalidMessage("Nop not a operation".to_string()),
            ws::Message::Pong(_) => Command::InvalidMessage("Pong not a operation".to_string()),
        }
    }
}

impl CompletionWebSeocket {

    async fn stuff(previous_messages: Vec<models::Message>, ctx: &mut ws::WebsocketContext<Self>) {

        let open_ai_messages: Vec<ChatMessage> = previous_messages
            .iter()
            .map(|message| ChatMessage::from(message.clone()))
            .collect();

        let open_ai_api_key = std::env::var("OPEN_AI_API_KEY").expect("OPEN_AI_API_KEY must be set");
        let client = Client::new(open_ai_api_key);

        let parameters = ChatCompletionParameters {
            model: "gpt-3.5-turbo".into(),
            messages: open_ai_messages,
            temperature: None,
            top_p: None,
            n: None,
            stop: None,
            max_tokens: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
        };

        let mut response_content = String::new();
        let mut completion_tokens = 0;
        let mut stream = client.chat().create_stream(parameters).await.unwrap();

        while let Some(response) = stream.next().await {
            let chat_content = response.unwrap().choices[0].delta.content.clone().unwrap();
            completion_tokens += 1;

            // tx.send(Ok(chat_content.into()))
            //     .await
            //     .map_err(|_e| DefaultError {
            //         message: "Error sending message to websocket".into(),
            //     })?;
            ctx.text(serde_json::to_string(&Response::ChatMessage(chat_content.clone())).unwrap());
            response_content.push_str(chat_content.clone().as_str());
        }

        let completion_message = models::Message::from_details(
            response_content,
            previous_messages[0].topic_id,
            (previous_messages.len() + 1).try_into().unwrap(),
            "assistant".into(),
            Some(0),
            Some(completion_tokens),
        );

        let completion_message = ChatCompletionDTO {
            completion_message,
            completion_tokens,
        };

    }

}


impl Actor for CompletionWebSeocket {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        ctx.run_interval(std::time::Duration::from_secs(1), |act, ctx| {
            if chrono::Utc::now().signed_duration_since(act.last_pong).num_seconds() > 10 {
                ctx.stop();
            }
        });
    }
}

/// Handler for ws::Message message
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for CompletionWebSeocket {

    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        let command: Command = match msg {
            Ok(message) => message.into(),
            Err(_) => Command::InvalidMessage("Invalid message".to_string()),
        };
        match command {
            Command::Ping => {
                log::info!("Ping received");
                self.last_pong = chrono::Utc::now();
                ctx.pong("Pong".as_bytes());
            }
            Command::Prompt(messages) => {
                log::info!("Prompt received");
                let fut = async move {
                    CompletionWebSeocket::stuff(messages, ctx).await;
                };
                let fut = actix::fut::wrap_future::<_, Self>(fut);
                self.spawn_handle = Some(ctx.spawn(fut));
            }
            Command::RegenerateMessage => {
                log::info!("Regenerate message received");
                todo!();
            }
            Command::ChangeTopic(topic_id) => {
                log::info!("Change topic received");
                if !user_owns_topic_query(self.user_id, topic_id, &self.pool) {
                    return ctx.text(serde_json::to_string(&Response::Error("User does not own topic".to_string())).unwrap());
                }
                let messages = get_messages_for_topic_query(topic_id, &self.pool);
                match &messages {
                    Ok(messages) => {
                        ctx.text(serde_json::to_string(&Response::Messages(messages.to_vec())).unwrap())
                    }
                    Err(err) => {
                        ctx.text(serde_json::to_string(err).unwrap())
                    }
                }
            }
            Command::Stop => {
                log::info!("Stop received");
                todo!();
            }
            Command::InvalidMessage(e) => {
                ctx.text(serde_json::to_string(&Response::Error(e.to_string())).unwrap())
            }
        }
    }
}
