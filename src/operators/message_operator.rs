use crate::diesel::prelude::*;
use crate::handlers::message_handler::StreamItem;
use crate::operators::topic_operator::get_topic_query;
use crate::{
    data::models::{Message, Pool},
    errors::DefaultError,
};
use actix_web::web;
use openai_dive::v1::api::Client;
use openai_dive::v1::resources::chat_completion::{ChatCompletionParameters, ChatMessage};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatCompletionDTO {
    pub completion_message: Message,
    pub completion_tokens: i32,
}

pub fn get_topic_messages(
    messages_topic_id: uuid::Uuid,
    pool: &web::Data<Pool>,
) -> Result<Vec<Message>, DefaultError> {
    use crate::data::schema::messages::dsl::*;

    let mut conn = pool.get().unwrap();

    let topic_messages = messages
        .filter(topic_id.eq(messages_topic_id))
        .filter(deleted.eq(false))
        .order(sort_order.asc())
        .load::<Message>(&mut conn)
        .map_err(|_db_error| DefaultError {
            message: "Error getting topic messages".into(),
        })?;

    Ok(topic_messages)
}

pub fn user_owns_topic_query(
    user_given_id: uuid::Uuid,
    topic_id: uuid::Uuid,
    pool: &web::Data<Pool>,
) -> bool {
    use crate::data::schema::topics::dsl::*;

    let mut conn = pool.get().unwrap();

    let topic = topics
        .filter(id.eq(topic_id))
        .filter(user_id.eq(user_id))
        .first::<crate::data::models::Topic>(&mut conn);
    if topic.is_err() {
        return false;
    }

    topic.unwrap().user_id == user_given_id
}

pub fn create_message_query(
    new_message: Message,
    pool: &web::Data<Pool>,
) -> Result<(), DefaultError> {
    use crate::data::schema::messages::dsl::*;

    let mut conn = pool.get().unwrap();

    diesel::insert_into(messages)
        .values(&new_message)
        .execute(&mut conn)
        .map_err(|_db_error| DefaultError {
            message: "Error creating message, try again".into(),
        })?;

    Ok(())
}

pub fn create_system_message(
    messages_topic_id: uuid::Uuid,
    pool: &web::Data<Pool>,
) -> Result<Message, DefaultError> {
    let topic = crate::operators::topic_operator::get_topic_query(messages_topic_id, pool)?;
    let system_message_content = format!(
        "We are going to practice lincoln douglas debate over \"{}\". You will be {}. We are speaking to a judge. After my messages, you will respond exactly as follows:\n\nfeedback: {{aggressive feedback on how my argument could be improved}}\ncounterargument: {{A simulated counterargument, including evidence, that I can respond to in order to further practice my skills}}",
        topic.resolution,
        if topic.side { "affirming the resolution" } else { "negating the resolution" }
    );

    let system_message = Message::from_details(
        system_message_content,
        topic.id,
        0,
        "system".into(),
        Some(0),
        Some(0),
    );

    Ok(system_message)
}

pub fn create_topic_message_query(
    previous_messages: Vec<Message>,
    new_message: Message,
    pool: &web::Data<Pool>,
) -> Result<Vec<Message>, DefaultError> {
    let mut ret_messages = previous_messages.clone();
    let mut new_message_copy = new_message.clone();
    let mut previous_messages_len = previous_messages.len();

    if previous_messages.len() == 0 {
        let system_message = create_system_message(new_message.topic_id, pool)?;
        create_message_query(system_message.clone(), pool)?;
        previous_messages_len += 1;
    }

    new_message_copy.sort_order = (previous_messages_len + 1).try_into().unwrap();

    create_message_query(new_message_copy.clone(), pool)?;
    ret_messages.push(new_message_copy);

    Ok(ret_messages)
}
pub fn get_messages_for_topic_query(
    message_topic_id: uuid::Uuid,
    pool: &web::Data<Pool>,
) -> Result<Vec<Message>, DefaultError> {
    use crate::data::schema::messages::dsl::*;

    let mut conn = pool.get().unwrap();

    messages
        .filter(topic_id.eq(message_topic_id))
        .filter(deleted.eq(false))
        .load::<Message>(&mut conn)
        .map_err(|_db_error| DefaultError {
            message: "This topic does not exist for the authenticated user".into(),
        })
}

pub async fn get_openai_completion(
    previous_messages: Vec<Message>,
    tx: mpsc::Sender<StreamItem>,
) -> Result<ChatCompletionDTO, DefaultError> {
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
        match response {
            Ok(chat_response) => {
                completion_tokens += 1;

                let chat_content = chat_response.choices[0].delta.content.clone().unwrap();
                let multi_use_chat_content = chat_content.clone();
                tx.send(Ok(chat_content.into()))
                    .await
                    .map_err(|_e| DefaultError {
                        message: "Error sending message to receiver".into(),
                    })?;
                response_content.push_str(multi_use_chat_content.clone().as_str());
            }
            Err(e) => log::error!("Error getting chat completion: {}", e),
        }
    }

    let completion_message = Message::from_details(
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

    Ok(completion_message)
}

pub fn delete_message_query(
    given_user_id: &uuid::Uuid,
    given_message_id: uuid::Uuid,
    given_topic_id: uuid::Uuid,
    pool: &web::Data<Pool>,
) -> Result<(), DefaultError> {
    use crate::data::schema::messages::dsl::*;

    let mut conn = pool.get().unwrap();

    match get_topic_query(given_topic_id, &pool) {
        Ok(topic) if topic.user_id != *given_user_id => {
            return Err(DefaultError {
                message: "Unauthorized".into(),
            })
        }
        Ok(_topic) => {}
        Err(e) => return Err(e),
    };

    let target_message: Message = messages
        .find(given_message_id)
        .first::<Message>(&mut conn)
        .map_err(|_db_error| DefaultError {
            message: "Error finding message".into(),
        })?;

    diesel::update(messages.filter(sort_order.ge(target_message.sort_order)))
        .set(deleted.eq(true))
        .execute(&mut conn)
        .map_err(|_| DefaultError {
            message: "Error deleting message".into(),
        })?;

    Ok(())
}
