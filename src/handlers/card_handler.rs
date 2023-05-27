use actix_web::{web, HttpResponse};
use qdrant_client::prelude::Payload;
use qdrant_client::qdrant::PointStruct;
use qdrant_client::qdrant::value::Kind;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::operators::card_operator::create_openai_embedding;
use crate::operators::card_operator::{
    get_point_by_id_query, get_qdrant_connection, retrieved_point_to_card_dto, search_card_query,
};

use super::auth_handler::LoggedUser;

#[derive(Serialize, Deserialize)]
pub struct CreateCardData {
    content: String,
    link: Option<String>,
}

pub async fn create_card(
    card: web::Json<CreateCardData>,
    user: LoggedUser,
) -> Result<HttpResponse, actix_web::Error> {
    let embedding_vector = create_openai_embedding(&card.content).await?;

    let qdrant = get_qdrant_connection()
        .await
        .map_err(|err| actix_web::error::ErrorBadRequest(err.message))?;

    let id_str = user.id.to_string();

    let payload: qdrant_client::prelude::Payload = json!(
        {
            "content": card.content.clone(),
            "user_id": id_str,
            "link": card.link,
            "created_at": chrono::Utc::now().to_rfc3339(),
            "upvoes": 0,
            "downvotes": 0,
        }
    )
    .try_into()
    .map_err(actix_web::error::ErrorBadRequest)?;

    let point = PointStruct::new(uuid::Uuid::new_v4().to_string(), embedding_vector, payload);
    qdrant
        .upsert_points_blocking("debate_cards".to_string(), vec![point], None)
        .await
        .map_err(actix_web::error::ErrorBadRequest)?;

    Ok(HttpResponse::NoContent().finish())
}

#[derive(Serialize, Deserialize)]
pub struct SearchCardData {
    content: String,
}

pub async fn search_card(
    data: web::Json<SearchCardData>,
    page: Option<web::Path<u64>>,
) -> Result<HttpResponse, actix_web::Error> {
    let page = page.map(|page| page.into_inner()).unwrap_or(1);
    let embedding_vector = create_openai_embedding(&data.content).await?;

    let cards = search_card_query(embedding_vector, page).await?;

    Ok(HttpResponse::Ok().json(cards))
}

pub async fn get_card_by_id(
    card_id: web::Path<uuid::Uuid>,
) -> Result<HttpResponse, actix_web::Error> {
    let card = match get_point_by_id_query(card_id.into_inner()).await? {
        Some(point) => retrieved_point_to_card_dto(point).await,
        None => return Ok(HttpResponse::BadRequest().finish()),
    };

    match card {
        Some(card) => Ok(HttpResponse::Ok().json(card)),
        None => Ok(HttpResponse::BadRequest().finish()),
    }
}

#[derive(Serialize, Deserialize)]
pub struct VoteCardData {
    card_id: uuid::Uuid,
    vote: bool,
}

pub async fn vote_card(
    data: web::Json<VoteCardData>,
) -> Result<HttpResponse, actix_web::Error> {
    let qdrant = get_qdrant_connection()
        .await
        .map_err(|err| actix_web::error::ErrorBadRequest(err.message))?;

    let card = match get_point_by_id_query(data.card_id).await? {
        Some(card) => card,
        None => return Ok(HttpResponse::NotFound().finish()),
    };

    let upvote_kind = card.payload
        .get("upvotes")
        .map(|v| v.kind.clone())
        .flatten();

    let downvote_kind = card.payload
        .get("downvotes")
        .map(|v| v.kind.clone())
        .flatten();

    let (upvotes, downvotes) = match (upvote_kind, downvote_kind) {
        (Some(Kind::IntegerValue(upvotes)), Some(Kind::IntegerValue(downvotes))) => (upvotes, downvotes),
        (_, _) => (0, 0)
    };

    let mut payload = card.payload.clone();

    if data.vote {
        payload.insert("upvotes".to_string(), (upvotes + 1).into());
    } else {
        payload.insert("downvotes".to_string(), (downvotes + 1).into());
    }

    let point = PointStruct::new(
        card.id.unwrap(),
        card.vectors.unwrap(),
        Payload::new_from_hashmap(payload)
    );

    qdrant
        .upsert_points_blocking("debate_cards".to_string(), vec![point], None)
        .await
        .map_err(actix_web::error::ErrorBadRequest)?;

    Ok(HttpResponse::NoContent().finish())
}
