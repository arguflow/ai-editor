use crate::data::models::{
    CardMetadataWithVotes, CardVote, SlimUser, UserDTOWithScore, UserDTOWithVotesAndCards,
    UserScore,
};
use crate::diesel::prelude::*;
use crate::handlers::user_handler::UpdateUserData;
use crate::{
    data::models::{Pool, User},
    errors::DefaultError,
};
use actix_web::web;
use diesel::sql_types::{Text, BigInt};
pub fn get_user_by_email_query(
    user_email: &String,
    pool: &web::Data<Pool>,
) -> Result<User, DefaultError> {
    use crate::data::schema::users::dsl::*;

    let mut conn = pool.get().unwrap();

    let user: Option<User> = users
        .filter(email.eq(user_email))
        .first::<User>(&mut conn)
        .optional()
        .map_err(|_| DefaultError {
            message: "Error loading user",
        })?;
    match user {
        Some(user) => Ok(user),
        None => Err(DefaultError {
            message: "User not found",
        }),
    }
}

pub fn get_user_by_username_query(
    user_name: &String,
    pool: &web::Data<Pool>,
) -> Result<User, DefaultError> {
    use crate::data::schema::users::dsl::*;

    let mut conn = pool.get().unwrap();

    let user: Option<User> = users
        .filter(username.eq(user_name))
        .first::<User>(&mut conn)
        .optional()
        .map_err(|_| DefaultError {
            message: "Error loading user",
        })?;
    match user {
        Some(user) => Ok(user),
        None => Err(DefaultError {
            message: "User not found",
        }),
    }
}

pub fn get_user_by_id_query(
    user_id: &uuid::Uuid,
    pool: &web::Data<Pool>,
) -> Result<User, DefaultError> {
    use crate::data::schema::users::dsl::*;

    let mut conn = pool.get().unwrap();

    let user: Option<User> = users
        .filter(id.eq(user_id))
        .first::<User>(&mut conn)
        .optional()
        .map_err(|_| DefaultError {
            message: "Error loading user",
        })?;
    match user {
        Some(user) => Ok(user),
        None => Err(DefaultError {
            message: "User not found",
        }),
    }
}

pub fn get_user_with_votes_and_cards_by_id_query(
    user_id: &uuid::Uuid,
    page: &i64,
    pool: &web::Data<Pool>,
) -> Result<UserDTOWithVotesAndCards, DefaultError> {
    use crate::data::schema::card_metadata::dsl as card_metadata_columns;
    use crate::data::schema::card_votes::dsl as card_votes_columns;
    use crate::data::schema::users::dsl as user_columns;

    let mut conn = pool.get().unwrap();

    let user_result: Option<User> = user_columns::users
        .filter(user_columns::id.eq(user_id))
        .first::<User>(&mut conn)
        .optional()
        .map_err(|_| DefaultError {
            message: "Error loading user",
        })?;
    let user = match user_result {
        Some(user) => Ok(user),
        None => Err(DefaultError {
            message: "User not found",
        }),
    }?;

    let user_card_metadatas = card_metadata_columns::card_metadata
        .filter(card_metadata_columns::author_id.eq(user.id))
        .order(card_metadata_columns::updated_at.desc())
        .limit(25)
        .offset((page - 1) * 25)
        .load::<crate::data::models::CardMetadata>(&mut conn)
        .map_err(|_| DefaultError {
            message: "Error loading user cards",
        })?;

    let card_votes: Vec<CardVote> = card_votes_columns::card_votes
        .filter(
            card_votes_columns::card_metadata_id.eq_any(
                user_card_metadatas
                    .iter()
                    .map(|metadata| metadata.id)
                    .collect::<Vec<uuid::Uuid>>(),
            ),
        )
        .load::<CardVote>(&mut conn)
        .map_err(|_| DefaultError {
            message: "Failed to load upvotes",
        })?;

    let card_metadata_with_upvotes: Vec<CardMetadataWithVotes> = (&user_card_metadatas)
        .into_iter()
        .map(|metadata| {
            let votes = card_votes
                .iter()
                .filter(|upvote| upvote.card_metadata_id == metadata.id)
                .collect::<Vec<&CardVote>>();
            let total_upvotes = votes.iter().filter(|upvote| upvote.vote).count() as i64;
            let total_downvotes = votes.iter().filter(|upvote| !upvote.vote).count() as i64;
            let vote_by_current_user = None;

            let author = None;

            CardMetadataWithVotes {
                id: metadata.id,
                content: metadata.content.clone(),
                link: metadata.link.clone(),
                author,
                qdrant_point_id: metadata.qdrant_point_id,
                total_upvotes,
                total_downvotes,
                vote_by_current_user,
                created_at: metadata.created_at,
                updated_at: metadata.updated_at,
            }
        })
        .collect();

    let user_card_votes = card_votes_columns::card_votes
        .filter(
            card_votes_columns::card_metadata_id.eq_any(
                user_card_metadatas
                    .iter()
                    .map(|metadata| metadata.id)
                    .collect::<Vec<uuid::Uuid>>(),
            ),
        )
        .load::<crate::data::models::CardVote>(&mut conn)
        .map_err(|_| DefaultError {
            message: "Failed to load upvotes",
        })?;
    let total_upvotes_received = user_card_votes
        .iter()
        .filter(|card_vote| card_vote.vote)
        .count() as i32;
    let total_downvotes_received = user_card_votes
        .iter()
        .filter(|card_vote| !card_vote.vote)
        .count() as i32;

    let total_votes_cast = card_votes_columns::card_votes
        .filter(card_votes_columns::voted_user_id.eq(user.id))
        .count()
        .get_result::<i64>(&mut conn)
        .map_err(|_| DefaultError {
            message: "Failed to load total votes cast",
        })? as i32;

    Ok(UserDTOWithVotesAndCards {
        id: user.id,
        email: if user.visible_email {
            Some(user.email)
        } else {
            None
        },
        username: user.username,
        website: user.website,
        visible_email: user.visible_email,
        created_at: user.created_at,
        cards: card_metadata_with_upvotes,
        total_upvotes_received,
        total_downvotes_received,
        total_votes_cast,
    })
}

pub fn update_user_query(
    user_id: &uuid::Uuid,
    new_user: &UpdateUserData,
    pool: &web::Data<Pool>,
) -> Result<SlimUser, DefaultError> {
    use crate::data::schema::users::dsl::*;

    let mut conn = pool.get().unwrap();

    if new_user.username.clone().unwrap_or("".to_string()) != "" {
        let user_by_username =
            get_user_by_username_query(&new_user.username.clone().unwrap(), pool);

        match user_by_username {
            Ok(old_user) => {
                if !(old_user.username.is_some()
                    && old_user.username.unwrap() == new_user.username.clone().unwrap())
                {
                    return Err(DefaultError {
                        message: "That username is already taken",
                    });
                }
            }
            Err(_) => {}
        }
    }

    let new_user_name: Option<String> = match new_user.username.clone() {
        Some(user_name) => {
            if user_name != "" {
                Some(user_name)
            } else {
                None
            }
        }
        None => None,
    };
    let new_user_website: Option<String> = match new_user.website.clone() {
        Some(user_website) => {
            if user_website != "" {
                Some(user_website)
            } else {
                None
            }
        }
        None => None,
    };

    let user: User = diesel::update(users.filter(id.eq(user_id)))
        .set((
            username.eq(&new_user_name),
            website.eq(&new_user_website),
            visible_email.eq(&new_user.visible_email),
        ))
        .get_result(&mut conn)
        .map_err(|_| DefaultError {
            message: "Error updating user",
        })?;

    Ok(SlimUser::from(user))
}

pub fn get_top_users_query(
    page: &i64,
    pool: &web::Data<Pool>,
) -> Result<Vec<UserDTOWithScore>, DefaultError> {
    use crate::data::schema::card_metadata::dsl as card_metadata_columns;
    use crate::data::schema::card_votes::dsl as card_votes_columns;
    use crate::data::schema::users::dsl as users_columns;

    let mut conn = pool.get().unwrap();

    let query = card_metadata_columns::card_metadata
        .inner_join(card_votes_columns::card_votes.on(card_metadata_columns::id.eq(card_votes_columns::card_metadata_id)))
        .select((
            card_metadata_columns::author_id,
            diesel::dsl::sql::<BigInt>("(SUM(case when vote = true then 1 else 0 end) - SUM(case when vote = false then 1 else 0 end)) as score"),
        ))
        .group_by((
            card_metadata_columns::author_id,
        ))
        .order(diesel::dsl::sql::<Text>("score desc"))
        .limit(25)
        .offset((page - 1) * 25);

    let user_scores: Vec<UserScore> =
        query
            .load::<UserScore>(&mut conn)
            .map_err(|_| DefaultError {
                message: "Failed to load top users",
            })?;

    let users_with_scores = users_columns::users
        .filter(
            users_columns::id.eq_any(
                user_scores
                    .iter()
                    .map(|user_score| user_score.author_id)
                    .collect::<Vec<uuid::Uuid>>(),
            ),
        )
        .load::<User>(&mut conn)
        .map_err(|_| DefaultError {
            message: "Failed to load top users",
        })?;

    let user_scores_with_users = user_scores
        .iter()
        .map(|user_score| {
            let user = users_with_scores
                .iter()
                .find(|user| user.id == user_score.author_id)
                .unwrap();

            UserDTOWithScore {
                id: user_score.author_id,
                email: if user.visible_email {
                    Some(user.email.clone())
                } else {
                    None
                },
                username: user.username.clone(),
                website: user.website.clone(),
                visible_email: user.visible_email,
                created_at: user.created_at,
                score: user_score.score,
            }
        })
        .collect::<Vec<UserDTOWithScore>>();

    Ok(user_scores_with_users)
}
