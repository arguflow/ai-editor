use crate::data::models::{
    CardMetadata, CardMetadataWithVotes, CardMetadataWithVotesWithoutScore, Pool,
};
use crate::errors::ServiceError;
use crate::operators::card_operator::{
    create_openai_embedding, delete_card_metadata_query, get_card_count_query,
    get_metadata_and_votes_from_id_query, get_metadata_from_point_ids, insert_card_metadata_query,
    insert_duplicate_card_metadata_query, search_full_text_card_query, update_card_metadata_query,
};
use crate::operators::card_operator::{
    get_metadata_from_id_query, get_qdrant_connection, search_card_query,
};
use actix_web::{web, HttpResponse};
use difference::{Changeset, Difference};
use qdrant_client::qdrant::points_selector::PointsSelectorOneOf;
use qdrant_client::qdrant::{PointStruct, PointsIdsList, PointsSelector};
use serde::{Deserialize, Serialize};
use serde_json::json;
use soup::Soup;

use super::auth_handler::LoggedUser;

#[derive(Serialize, Deserialize)]
pub struct CreateCardData {
    content: String,
    card_html: Option<String>,
    link: Option<String>,
    oc_file_path: Option<String>,
    private: Option<bool>,
}

pub async fn create_card(
    card: web::Json<CreateCardData>,
    pool: web::Data<Pool>,
    user: LoggedUser,
) -> Result<HttpResponse, actix_web::Error> {
    let mut private = card.private.unwrap_or(false);
    let mut collision: Option<uuid::Uuid> = None;

    let words_in_content = card.content.split(' ').collect::<Vec<&str>>().len();
    if words_in_content < 70 {
        return Ok(HttpResponse::BadRequest().json(json!({
            "message": "Card content must be at least 70 words long",
        })));
    }

    let embedding_vector = create_openai_embedding(&card.content).await?;

    let cards = search_card_query(embedding_vector.clone(), 1, pool.clone(), None, None)
        .await
        .map_err(|err| ServiceError::BadRequest(err.message.into()))?;
    let first_result = cards.search_results.get(0);

    if let Some(score_card) = first_result {
        let mut similarity_threashold = 0.95;
        if card.content.len() < 200 {
            similarity_threashold = 0.9;
        }

        if score_card.score >= similarity_threashold {
            //Sets collision to collided card id and forces private
            collision = Some(score_card.point_id);
            private = true;
        }
    }

    //if collision is not nil, insert card with collision
    if collision.is_some() {
        web::block(move || {
            insert_duplicate_card_metadata_query(
                CardMetadata::from_details(
                    &card.content,
                    &card.card_html,
                    &card.link,
                    &card.oc_file_path,
                    user.id,
                    None,
                    true,
                ),
                collision.unwrap(),
                &pool,
            )
        })
        .await?
        .map_err(|err| ServiceError::BadRequest(err.message.into()))?;
    } else {
        let qdrant = get_qdrant_connection()
            .await
            .map_err(|err| ServiceError::BadRequest(err.message.into()))?;
        //if private is true, set payload to private
        let payload: qdrant_client::prelude::Payload = if private {
            json!({"private": true}).try_into().unwrap()
        } else {
            json!({}).try_into().unwrap()
        };

        let point_id = uuid::Uuid::new_v4();
        let point = PointStruct::new(point_id.clone().to_string(), embedding_vector, payload);

        web::block(move || {
            insert_card_metadata_query(
                CardMetadata::from_details(
                    &card.content,
                    &card.card_html,
                    &card.link,
                    &card.oc_file_path,
                    user.id,
                    Some(point_id),
                    private,
                ),
                &pool,
            )
        })
        .await?
        .map_err(|err| ServiceError::BadRequest(err.message.into()))?;

        qdrant
            .upsert_points_blocking("debate_cards".to_string(), vec![point], None)
            .await
            .map_err(|_err| ServiceError::BadRequest("Failed inserting card to qdrant".into()))?;
    }

    Ok(HttpResponse::NoContent().finish())
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DeleteCardData {
    card_uuid: uuid::Uuid,
}

pub async fn delete_card(
    card: web::Json<DeleteCardData>,
    pool: web::Data<Pool>,
    user: LoggedUser,
) -> Result<HttpResponse, actix_web::Error> {
    let card1 = card.clone();
    let pool1 = pool.clone();
    let card_metadata = web::block(move || get_metadata_from_id_query(card1.card_uuid, pool1))
        .await?
        .map_err(|err| ServiceError::BadRequest(err.message.into()))?;
    if user.id != card_metadata.author_id {
        return Err(ServiceError::Unauthorized.into());
    }
    let qdrant = get_qdrant_connection()
        .await
        .map_err(|err| ServiceError::BadRequest(err.message.into()))?;
    let deleted_values = PointsSelector {
        points_selector_one_of: Some(PointsSelectorOneOf::Points(PointsIdsList {
            ids: vec![card_metadata
                .qdrant_point_id
                .unwrap_or(uuid::Uuid::nil())
                .to_string()
                .into()],
        })),
    };
    web::block(move || delete_card_metadata_query(&card.card_uuid, &pool))
        .await?
        .map_err(|err| ServiceError::BadRequest(err.message.into()))?;

    qdrant
        .delete_points_blocking("debate_cards".to_string(), &deleted_values, None)
        .await
        .map_err(|_err| ServiceError::BadRequest("Failed deleting card from qdrant".into()))?;
    Ok(HttpResponse::NoContent().finish())
}

#[derive(Serialize, Deserialize, Clone)]
pub struct UpdateCardData {
    card_uuid: uuid::Uuid,
    link: Option<String>,
    card_html: Option<String>,
    private: Option<bool>,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct CardHtmlUpdateError {
    pub message: String,
    changed_content: String,
}
pub async fn update_card(
    card: web::Json<UpdateCardData>,
    pool: web::Data<Pool>,
    user: LoggedUser,
) -> Result<HttpResponse, actix_web::Error> {
    let card1 = card.clone();
    let pool1 = pool.clone();
    let card_metadata = web::block(move || get_metadata_from_id_query(card1.card_uuid, pool1))
        .await?
        .map_err(|err| ServiceError::BadRequest(err.message.into()))?;
    if user.id != card_metadata.author_id {
        return Err(ServiceError::Unauthorized.into());
    }

    let link = card
        .link
        .clone()
        .unwrap_or_else(|| card_metadata.link.unwrap_or_default());

    let soup = Soup::new(card.card_html.as_ref().unwrap_or(&"".to_string()).as_str());
    if soup.text() != card_metadata.content && card_metadata.card_html.is_some() {
        let soup_text_ref = soup.text();
        let Changeset { diffs, .. } = Changeset::new(&card_metadata.content, &soup_text_ref, " ");
        let mut ret: String = Default::default();
        for item in &diffs {
            match item {
                Difference::Same(ref x) => {
                    ret += format!(" {}", x).as_str();
                }
                Difference::Add(ref x) => {
                    ret += format!("++++{}", x).as_str();
                }
                Difference::Rem(ref x) => {
                    ret += format!("----{}", x).as_str();
                }
            }
        }

        return Ok(HttpResponse::BadRequest().json(CardHtmlUpdateError {
            message: "Card content has changed".to_string(),
            changed_content: ret,
        }));
    }

    let card_html = match card.card_html.clone() {
        Some(card_html) => Some(card_html),
        None => card_metadata.card_html,
    };

    if card_metadata.private
        && !card.private.unwrap_or(true)
        && card_metadata.qdrant_point_id.is_none()
    {
        return Err(ServiceError::BadRequest("Cannot make a duplicate card public".into()).into());
    }
    let private = card.private.unwrap_or(card_metadata.private);

    web::block(move || {
        update_card_metadata_query(
            CardMetadata::from_details_with_id(
                card.card_uuid,
                &card_metadata.content,
                &card_html,
                &Some(link),
                &card_metadata.oc_file_path,
                user.id,
                card_metadata.qdrant_point_id,
                private,
            ),
            &pool,
        )
    })
    .await?
    .map_err(|err| ServiceError::BadRequest(err.message.into()))?;

    Ok(HttpResponse::NoContent().finish())
}
#[derive(Serialize, Deserialize)]
pub struct SearchCardData {
    content: String,
    filter_oc_file_path: Option<Vec<String>>,
    filter_link_url: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
pub struct ScoreCardDTO {
    metadata: CardMetadataWithVotesWithoutScore,
    score: f32,
}

#[derive(Serialize, Deserialize)]
pub struct SearchCardQueryResponseBody {
    score_cards: Vec<ScoreCardDTO>,
    total_card_pages: i64,
}

pub async fn search_card(
    data: web::Json<SearchCardData>,
    page: Option<web::Path<u64>>,
    user: Option<LoggedUser>,
    pool: web::Data<Pool>,
) -> Result<HttpResponse, actix_web::Error> {
    //search over the links as well
    let page = page.map(|page| page.into_inner()).unwrap_or(1);
    let embedding_vector = create_openai_embedding(&data.content).await?;
    let pool2 = pool.clone();
    let search_card_query_results = search_card_query(
        embedding_vector,
        page,
        pool,
        data.filter_oc_file_path.clone(),
        data.filter_link_url.clone(),
    )
    .await
    .map_err(|err| ServiceError::BadRequest(err.message.into()))?;

    let point_ids = search_card_query_results
        .search_results
        .iter()
        .map(|point| point.point_id)
        .collect::<Vec<_>>();

    let current_user_id = user.map(|user| user.id);
    let metadata_cards =
        web::block(move || get_metadata_from_point_ids(point_ids, current_user_id, pool2))
            .await?
            .map_err(|err| ServiceError::BadRequest(err.message.into()))?;

    let score_cards: Vec<ScoreCardDTO> = search_card_query_results
        .search_results
        .iter()
        .map(|search_result| {
            let card = metadata_cards
                .iter()
                .find(|metadata_card| metadata_card.qdrant_point_id == search_result.point_id)
                .unwrap();

            ScoreCardDTO {
                metadata: <CardMetadataWithVotes as Into<CardMetadataWithVotesWithoutScore>>::into(
                    (*card).clone(),
                ),
                score: search_result.score,
            }
        })
        .collect();

    Ok(HttpResponse::Ok().json(SearchCardQueryResponseBody {
        score_cards,
        total_card_pages: search_card_query_results.total_card_pages,
    }))
}

pub async fn search_full_text_card(
    data: web::Json<SearchCardData>,
    page: Option<web::Path<u64>>,
    user: Option<LoggedUser>,
    pool: web::Data<Pool>,
) -> Result<HttpResponse, actix_web::Error> {
    //search over the links as well
    let page = page.map(|page| page.into_inner()).unwrap_or(1);
    let current_user_id = user.map(|user| user.id);
    let search_results_result = search_full_text_card_query(
        data.content.clone(),
        page,
        pool,
        current_user_id,
        data.filter_oc_file_path.clone(),
        data.filter_link_url.clone(),
    );

    let search_card_query_results = match search_results_result {
        Ok(results) => results,
        Err(err) => return Ok(HttpResponse::BadRequest().json(err)),
    };

    let full_text_cards: Vec<ScoreCardDTO> = search_card_query_results
        .search_results
        .iter()
        .map(|search_result| ScoreCardDTO {
            metadata: <CardMetadataWithVotes as Into<CardMetadataWithVotesWithoutScore>>::into(
                search_result.clone(),
            ),
            score: search_result.score.unwrap_or(0.0),
        })
        .collect();

    Ok(HttpResponse::Ok().json(SearchCardQueryResponseBody {
        score_cards: full_text_cards,
        total_card_pages: search_card_query_results.total_card_pages,
    }))
}

pub async fn get_card_by_id(
    card_id: web::Path<uuid::Uuid>,
    user: Option<LoggedUser>,
    pool: web::Data<Pool>,
) -> Result<HttpResponse, actix_web::Error> {
    let current_user_id = user.map(|user| user.id);
    let card = web::block(move || {
        get_metadata_and_votes_from_id_query(card_id.into_inner(), current_user_id, pool)
    })
    .await?
    .map_err(|err| ServiceError::BadRequest(err.message.into()))?;
    if card.private && current_user_id.is_none() {
        return Ok(HttpResponse::Unauthorized()
            .json(json!({"message": "You must be signed in to view this card"})));
    }
    if card.private && Some(card.clone().author.unwrap().id) != current_user_id {
        return Ok(HttpResponse::Forbidden()
            .json(json!({"message": "You are not authorized to view this card"})));
    }
    Ok(HttpResponse::Ok().json(card))
}

pub async fn get_total_card_count(pool: web::Data<Pool>) -> Result<HttpResponse, actix_web::Error> {
    let total_count = web::block(move || get_card_count_query(&pool))
        .await?
        .map_err(|err| ServiceError::BadRequest(err.message.into()))?;

    Ok(HttpResponse::Ok().json(json!({ "total_count": total_count })))
}
