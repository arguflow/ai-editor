use crate::data::models::CardCollisions;
use crate::data::models::CardMetadataWithVotes;
use crate::data::models::CardVote;
use crate::data::models::FullTextSearchResult;
use crate::data::models::User;
use crate::data::models::UserDTO;
use crate::data::schema::card_collisions;
use crate::diesel::TextExpressionMethods;
use crate::diesel::{ExpressionMethods, QueryDsl, RunQueryDsl};
use crate::{
    data::models::{CardMetadata, Pool},
    errors::DefaultError,
};
use actix_web::web;

use diesel::dsl::sql;
use diesel::sql_types::Bool;
use diesel::sql_types::Float;
use diesel::sql_types::Nullable;
use diesel::sql_types::Text;
use openai_dive::v1::{api::Client, resources::embedding::EmbeddingParameters};
use qdrant_client::qdrant::condition::ConditionOneOf::HasId;
use qdrant_client::{
    prelude::{QdrantClient, QdrantClientConfig},
    qdrant::{point_id::PointIdOptions, Condition, Filter, HasIdCondition, PointId, SearchPoints},
};
use serde::{Deserialize, Serialize};

pub async fn get_qdrant_connection() -> Result<QdrantClient, DefaultError> {
    let qdrant_url = std::env::var("QDRANT_URL").expect("QDRANT_URL must be set");
    QdrantClient::new(Some(QdrantClientConfig::from_url(qdrant_url.as_str())))
        .await
        .map_err(|_err| DefaultError {
            message: "Failed to connect to Qdrant",
        })
}

pub async fn create_openai_embedding(message: &str) -> Result<Vec<f32>, actix_web::Error> {
    let open_ai_api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set");
    let client = Client::new(open_ai_api_key);

    // Vectorize
    let parameters = EmbeddingParameters {
        model: "text-embedding-ada-002".to_string(),
        input: message.to_string(),
    };

    let embeddings = client
        .embeddings()
        .create(parameters)
        .await
        .map_err(actix_web::error::ErrorBadRequest)?;

    let vector = embeddings.data.get(0).unwrap().embedding.clone();
    Ok(vector.iter().map(|&x| x as f32).collect())
}

#[derive(Serialize, Deserialize)]
pub struct SearchResult {
    pub score: f32,
    pub point_id: uuid::Uuid,
}

#[derive(Serialize, Deserialize)]
pub struct SearchCardQueryResult {
    pub search_results: Vec<SearchResult>,
    pub total_card_pages: i64,
}

pub async fn search_card_query(
    embedding_vector: Vec<f32>,
    page: u64,
    pool: web::Data<Pool>,
    filter_oc_file_path: Option<Vec<String>>,
    filter_link_url: Option<Vec<String>>,
) -> Result<SearchCardQueryResult, DefaultError> {
    let page = if page == 0 { 1 } else { page };
    use crate::data::schema::card_metadata::dsl as card_metadata_columns;
    let mut conn = pool.get().unwrap();
    let mut query = card_metadata_columns::card_metadata
        .select(card_metadata_columns::qdrant_point_id)
        .filter(card_metadata_columns::private.eq(false))
        .into_boxed();
    let filter_oc_file_path = filter_oc_file_path.unwrap_or([].to_vec());
    let filter_link_url = filter_link_url.unwrap_or([].to_vec());

    if !filter_oc_file_path.is_empty() {
        query = query.filter(
            card_metadata_columns::oc_file_path
                .like(format!("%{}%", filter_oc_file_path.get(0).unwrap())),
        );
    }

    for file_path in filter_oc_file_path.iter().skip(1) {
        query =
            query.or_filter(card_metadata_columns::oc_file_path.like(format!("%{}%", file_path)));
    }

    if !filter_link_url.is_empty() {
        query = query.filter(
            card_metadata_columns::link.like(format!("%{}%", filter_link_url.get(0).unwrap())),
        );
    }
    for link_url in filter_link_url.iter().skip(1) {
        query = query.or_filter(card_metadata_columns::link.like(format!("%{}%", link_url)));
    }

    let filtered_ids: Vec<Option<uuid::Uuid>> =
        query.load(&mut conn).map_err(|_| DefaultError {
            message: "Failed to load metadata",
        })?;

    let qdrant = get_qdrant_connection().await?;

    let filtered_point_ids: &Vec<PointId> = &filtered_ids
        .iter()
        .map(|uuid| uuid.unwrap_or(uuid::Uuid::nil()).to_string().into())
        .collect::<Vec<PointId>>();

    let mut filter = Filter::default();
    filter.should.push(Condition {
        condition_one_of: Some(HasId(HasIdCondition {
            has_id: (filtered_point_ids).to_vec(),
        })),
    });

    let data = qdrant
        .search_points(&SearchPoints {
            collection_name: "debate_cards".to_string(),
            vector: embedding_vector,
            limit: 25,
            offset: Some((page - 1) * 25),
            with_payload: None,
            filter: Some(filter),
            ..Default::default()
        })
        .await
        .map_err(|_e| DefaultError {
            message: "Failed to search points on Qdrant",
        })?;

    let point_ids: Vec<SearchResult> = data
        .result
        .iter()
        .filter_map(|point| match point.clone().id?.point_id_options? {
            PointIdOptions::Uuid(id) => Some(SearchResult {
                score: point.score,
                point_id: uuid::Uuid::parse_str(&id).ok()?,
            }),
            PointIdOptions::Num(_) => None,
        })
        .collect();

    Ok(SearchCardQueryResult {
        search_results: point_ids,
        total_card_pages: (filtered_point_ids.len() as f64 / 25.0).ceil() as i64,
    })
}

fn get_metadata(
    card_metadata: Vec<FullTextSearchResult>,
    current_user_id: Option<uuid::Uuid>,
    mut conn: r2d2::PooledConnection<diesel::r2d2::ConnectionManager<diesel::PgConnection>>,
) -> Result<Vec<CardMetadataWithVotes>, DefaultError> {
    use crate::data::schema::card_votes::dsl as card_votes_columns;
    use crate::data::schema::users::dsl as user_columns;
    let card_creators: Vec<User> = user_columns::users
        .filter(
            user_columns::id.eq_any(
                card_metadata
                    .iter()
                    .map(|metadata| metadata.author_id)
                    .collect::<Vec<uuid::Uuid>>(),
            ),
        )
        .load::<User>(&mut conn)
        .map_err(|_| DefaultError {
            message: "Failed to load card creators",
        })?;

    let card_votes: Vec<CardVote> = card_votes_columns::card_votes
        .filter(
            card_votes_columns::card_metadata_id.eq_any(
                card_metadata
                    .iter()
                    .map(|metadata| metadata.id)
                    .collect::<Vec<uuid::Uuid>>(),
            ),
        )
        .load::<CardVote>(&mut conn)
        .map_err(|_| DefaultError {
            message: "Failed to load upvotes",
        })?;

    let card_metadata_with_upvotes: Vec<CardMetadataWithVotes> = card_metadata
        .into_iter()
        .map(|metadata| {
            let votes = card_votes
                .iter()
                .filter(|upvote| upvote.card_metadata_id == metadata.id)
                .collect::<Vec<&CardVote>>();
            let total_upvotes = votes.iter().filter(|upvote| upvote.vote).count() as i64;
            let total_downvotes = votes.iter().filter(|upvote| !upvote.vote).count() as i64;
            let vote_by_current_user = match current_user_id {
                Some(user_id) => votes
                    .iter()
                    .find(|upvote| upvote.voted_user_id == user_id)
                    .map(|upvote| upvote.vote),
                None => None,
            };

            let author = card_creators
                .iter()
                .find(|user| user.id == metadata.author_id)
                .map(|user| UserDTO {
                    id: user.id,
                    username: user.username.clone(),
                    email: if user.visible_email {
                        Some(user.email.clone())
                    } else {
                        None
                    },
                    website: user.website.clone(),
                    visible_email: user.visible_email,
                    created_at: user.created_at,
                });

            CardMetadataWithVotes {
                id: metadata.id,
                content: metadata.content,
                link: metadata.link,
                oc_file_path: metadata.oc_file_path,
                author,
                qdrant_point_id: metadata.qdrant_point_id.unwrap_or(uuid::Uuid::nil()),
                total_upvotes,
                total_downvotes,
                vote_by_current_user,
                created_at: metadata.created_at,
                updated_at: metadata.updated_at,
                private: metadata.private,
                score: metadata.score,
            }
        })
        .collect();
    Ok(card_metadata_with_upvotes)
}

#[derive(Serialize, Deserialize)]
pub struct FullTextSearchCardQueryResult {
    pub search_results: Vec<CardMetadataWithVotes>,
    pub total_card_pages: i64,
}

pub fn search_full_text_card_query(
    user_query: String,
    page: u64,
    pool: web::Data<Pool>,
    current_user_id: Option<uuid::Uuid>,
    filter_oc_file_path: Option<Vec<String>>,
    filter_link_url: Option<Vec<String>>,
) -> Result<FullTextSearchCardQueryResult, DefaultError> {
    let page = if page == 0 { 1 } else { page };
    use crate::data::schema::card_metadata::dsl as card_metadata_columns;
    let mut conn = pool.get().unwrap();
    let mut query = card_metadata_columns::card_metadata
        .select((
            card_metadata_columns::id,
            card_metadata_columns::content,
            card_metadata_columns::link,
            card_metadata_columns::author_id,
            card_metadata_columns::qdrant_point_id,
            card_metadata_columns::created_at,
            card_metadata_columns::updated_at,
            card_metadata_columns::oc_file_path,
            card_metadata_columns::card_html,
            card_metadata_columns::private,
            sql::<Nullable<Float>>("(ts_rank(card_metadata_tsvector, to_tsquery('english', ")
                .bind::<Text, _>(
                    user_query
                        .split_whitespace()
                        .collect::<Vec<&str>>()
                        .join(" & "),
                )
                .sql(") , 32) * 10) AS rank"),
        ))
        .filter(card_metadata_columns::private.eq(false))
        .into_boxed();

    query = query.filter(
        sql::<Bool>("card_metadata_tsvector @@ to_tsquery('english', ")
            .bind::<Text, _>(
                user_query
                    .split_whitespace()
                    .collect::<Vec<&str>>()
                    .join(" & "),
            )
            .sql(")"),
    );

    let filter_oc_file_path = filter_oc_file_path.unwrap_or([].to_vec());
    let filter_link_url = filter_link_url.unwrap_or([].to_vec());

    if !filter_oc_file_path.is_empty() {
        query = query.filter(
            card_metadata_columns::oc_file_path
                .like(format!("%{}%", filter_oc_file_path.get(0).unwrap())),
        );
    }

    for file_path in filter_oc_file_path.iter().skip(1) {
        query =
            query.or_filter(card_metadata_columns::oc_file_path.like(format!("%{}%", file_path)));
    }

    if !filter_link_url.is_empty() {
        query = query.filter(
            card_metadata_columns::link.like(format!("%{}%", filter_link_url.get(0).unwrap())),
        );
    }
    for link_url in filter_link_url.iter().skip(1) {
        query = query.or_filter(card_metadata_columns::link.like(format!("%{}%", link_url)));
    }

    query = query
        .order(sql::<Text>("rank DESC"))
        .limit(25)
        .offset(((page - 1) * 25).try_into().unwrap());

    let searched_cards: Vec<FullTextSearchResult> =
        query.load(&mut conn).map_err(|_| DefaultError {
            message: "Failed to load searched cards",
        })?;

    let card_metadata_with_upvotes = get_metadata(searched_cards.clone(), current_user_id, conn)
        .map_err(|_| DefaultError {
            message: "Failed to load searched cards",
        })?;
    Ok(FullTextSearchCardQueryResult {
        search_results: card_metadata_with_upvotes,
        total_card_pages: (searched_cards.len() as f64 / 25.0).ceil() as i64,
    })
}
#[derive(Serialize, Deserialize)]
pub struct ScoredCardDTO {
    pub metadata: CardMetadata,
    pub score: f32,
}

pub fn get_metadata_from_point_ids(
    point_ids: Vec<uuid::Uuid>,
    current_user_id: Option<uuid::Uuid>,
    pool: web::Data<Pool>,
) -> Result<Vec<CardMetadataWithVotes>, DefaultError> {
    use crate::data::schema::card_metadata::dsl as card_metadata_columns;

    let mut conn = pool.get().unwrap();

    let card_metadata: Vec<CardMetadata> = card_metadata_columns::card_metadata
        .filter(card_metadata_columns::qdrant_point_id.eq_any(point_ids))
        .select((
            card_metadata_columns::id,
            card_metadata_columns::content,
            card_metadata_columns::link,
            card_metadata_columns::author_id,
            card_metadata_columns::qdrant_point_id,
            card_metadata_columns::created_at,
            card_metadata_columns::updated_at,
            card_metadata_columns::oc_file_path,
            card_metadata_columns::card_html,
            card_metadata_columns::private,
        ))
        .load::<CardMetadata>(&mut conn)
        .map_err(|_| DefaultError {
            message: "Failed to load metadata",
        })?;

    let converted_cards: Vec<FullTextSearchResult> = card_metadata
        .iter()
        .map(|card| <CardMetadata as Into<FullTextSearchResult>>::into(card.clone()))
        .collect::<Vec<FullTextSearchResult>>();

    let card_metadata_with_upvotes =
        get_metadata(converted_cards, current_user_id, conn).map_err(|_| DefaultError {
            message: "Failed to load metadata",
        })?;
    Ok(card_metadata_with_upvotes)
}

pub fn get_metadata_from_id_query(
    card_id: uuid::Uuid,
    pool: web::Data<Pool>,
) -> Result<CardMetadata, DefaultError> {
    use crate::data::schema::card_metadata::dsl as card_metadata_columns;

    let mut conn = pool.get().unwrap();

    card_metadata_columns::card_metadata
        .filter(card_metadata_columns::id.eq(card_id))
        .select((
            card_metadata_columns::id,
            card_metadata_columns::content,
            card_metadata_columns::link,
            card_metadata_columns::author_id,
            card_metadata_columns::qdrant_point_id,
            card_metadata_columns::created_at,
            card_metadata_columns::updated_at,
            card_metadata_columns::oc_file_path,
            card_metadata_columns::card_html,
            card_metadata_columns::private,
        ))
        .first::<CardMetadata>(&mut conn)
        .map_err(|_| DefaultError {
            message: "Failed to load metadata",
        })
}

pub fn get_metadata_and_votes_from_id_query(
    card_id: uuid::Uuid,
    current_user_id: Option<uuid::Uuid>,
    pool: web::Data<Pool>,
) -> Result<CardMetadataWithVotes, DefaultError> {
    use crate::data::schema::card_metadata::dsl as card_metadata_columns;

    let mut conn = pool.get().unwrap();

    let card_metadata = card_metadata_columns::card_metadata
        .filter(card_metadata_columns::id.eq(card_id))
        .select((
            card_metadata_columns::id,
            card_metadata_columns::content,
            card_metadata_columns::link,
            card_metadata_columns::author_id,
            card_metadata_columns::qdrant_point_id,
            card_metadata_columns::created_at,
            card_metadata_columns::updated_at,
            card_metadata_columns::oc_file_path,
            card_metadata_columns::card_html,
            card_metadata_columns::private,
        ))
        .first::<CardMetadata>(&mut conn)
        .map_err(|_| DefaultError {
            message: "Failed to load metadata",
        })?;
    let converted_card: FullTextSearchResult =
        <CardMetadata as Into<FullTextSearchResult>>::into(card_metadata);

    let card_metadata_with_upvotes = get_metadata(vec![converted_card], current_user_id, conn)
        .map_err(|_| DefaultError {
            message: "Failed to load metadata",
        })?;
    Ok(card_metadata_with_upvotes.first().unwrap().clone())
}

pub fn insert_card_metadata_query(
    card_data: CardMetadata,
    pool: &web::Data<Pool>,
) -> Result<(), DefaultError> {
    use crate::data::schema::card_metadata::dsl::*;

    let mut conn = pool.get().unwrap();

    diesel::insert_into(card_metadata)
        .values(&card_data)
        .execute(&mut conn)
        .map_err(|_err| DefaultError {
            message: "Failed to insert card metadata",
        })?;

    Ok(())
}

pub fn insert_duplicate_card_metadata_query(
    card_data: CardMetadata,
    duplicate_card: uuid::Uuid,
    pool: &web::Data<Pool>,
) -> Result<(), DefaultError> {
    use crate::data::schema::card_collisions::dsl::*;
    use crate::data::schema::card_metadata::dsl::*;

    let mut conn = pool.get().unwrap();

    diesel::insert_into(card_metadata)
        .values(&card_data)
        .execute(&mut conn)
        .map_err(|_err| DefaultError {
            message: "Failed to insert card metadata",
        })?;

    //insert duplicate into card_collisions
    diesel::insert_into(card_collisions)
        .values(&CardCollisions::from_details(card_data.id, duplicate_card))
        .execute(&mut conn)
        .map_err(|_err| DefaultError {
            message: "Failed to insert card duplicate",
        })?;
    Ok(())
}

pub fn update_card_metadata_query(
    card_data: CardMetadata,
    pool: &web::Data<Pool>,
) -> Result<(), DefaultError> {
    use crate::data::schema::card_metadata::dsl as card_metadata_columns;
    use crate::data::schema::card_votes::dsl as card_votes_columns;

    let mut conn = pool.get().unwrap();

    diesel::update(
        card_metadata_columns::card_metadata.filter(card_metadata_columns::id.eq(card_data.id)),
    )
    .set((
        card_metadata_columns::link.eq(card_data.link),
        card_metadata_columns::card_html.eq(card_data.card_html),
        card_metadata_columns::private.eq(card_data.private),
    ))
    .execute(&mut conn)
    .map_err(|_err| DefaultError {
        message: "Failed to update card metadata",
    })?;
    diesel::update(
        card_votes_columns::card_votes
            .filter(card_votes_columns::card_metadata_id.eq(card_data.id)),
    )
    .set(card_votes_columns::deleted.eq(card_data.private))
    .execute(&mut conn)
    .map_err(|_err| DefaultError {
        message: "Failed to update card votes",
    })?;

    Ok(())
}

pub fn delete_card_metadata_query(
    card_uuid: &uuid::Uuid,
    pool: &web::Data<Pool>,
) -> Result<(), DefaultError> {
    use crate::data::schema::card_collisions::dsl as card_collisions_columns;
    use crate::data::schema::card_metadata::dsl as card_metadata_columns;
    let mut conn = pool.get().unwrap();

    diesel::delete(
        card_collisions_columns::card_collisions.filter(card_collisions::card_id.eq(card_uuid)),
    )
    .execute(&mut conn)
    .map_err(|_err| DefaultError {
        message: "Failed to delete card collusion",
    })?;

    diesel::delete(
        card_metadata_columns::card_metadata.filter(card_metadata_columns::id.eq(card_uuid)),
    )
    .execute(&mut conn)
    .map_err(|_err| DefaultError {
        message: "Failed to delete card metadata",
    })?;

    Ok(())
}

pub fn get_card_count_query(pool: &web::Data<Pool>) -> Result<i64, DefaultError> {
    use crate::data::schema::card_metadata::dsl::*;

    let mut conn = pool.get().unwrap();

    card_metadata
        .count()
        .get_result::<i64>(&mut conn)
        .map_err(|_err| DefaultError {
            message: "Failed to get card count",
        })
}

pub fn update_card_html_by_qdrant_point_id_query(
    point_id: &uuid::Uuid,
    new_card_html: &Option<String>,
    pool: &web::Data<Pool>,
) -> Result<(), DefaultError> {
    use crate::data::schema::card_metadata::dsl::*;

    let mut conn = pool.get().unwrap();

    diesel::update(
        card_metadata
            .filter(qdrant_point_id.eq(point_id))
            .filter(card_html.is_null()),
    )
    .set(card_html.eq(new_card_html))
    .execute(&mut conn)
    .map_err(|_err| DefaultError {
        message: "Failed to update card html",
    })?;

    Ok(())
}
