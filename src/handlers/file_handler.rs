use std::sync::{Arc, Mutex};

use crate::{
    data::models::{File, Pool},
    errors::ServiceError,
    operators::file_operator::{
        convert_docx_to_html_query, delete_file_query, get_file_query, get_user_file_query,
        get_user_id_of_file_query, update_file_query, CoreCard,
    },
};
use actix_web::{web, HttpResponse};
use base64::{
    alphabet,
    engine::{self, general_purpose},
    Engine as _,
};
use serde::{Deserialize, Serialize};

use super::auth_handler::LoggedUser;
pub async fn user_owns_file(
    user_id: uuid::Uuid,
    file_id: uuid::Uuid,
    pool: Arc<Mutex<web::Data<r2d2::Pool<diesel::r2d2::ConnectionManager<diesel::PgConnection>>>>>,
) -> Result<(), actix_web::Error> {
    let author_id = web::block(move || get_user_id_of_file_query(file_id, pool.lock().unwrap()))
        .await?
        .map_err(|err| ServiceError::BadRequest(err.message.into()))?;

    if author_id != user_id {
        return Err(ServiceError::Forbidden.into());
    }
    Ok(())
}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UploadFileData {
    pub base64_docx_file: String,
    pub file_name: String,
    pub file_mime_type: String,
    pub private: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UploadFileResult {
    pub file_metadata: File,
    pub collection_id: uuid::Uuid,
    pub created_cards: Vec<CoreCard>,
    pub rejected_cards: Vec<CoreCard>,
}

pub async fn upload_file_handler(
    data: web::Json<UploadFileData>,
    pool: web::Data<Pool>,
    user: LoggedUser,
) -> Result<HttpResponse, actix_web::Error> {
    let upload_file_data = data.into_inner();
    let pool_inner = pool.clone();

    let base64_engine = engine::GeneralPurpose::new(&alphabet::URL_SAFE, general_purpose::NO_PAD);

    let decoded_file_data = base64_engine
        .decode(upload_file_data.base64_docx_file)
        .map_err(|_e| ServiceError::BadRequest("Could not decode base64 file".to_string()))?;
    let private = upload_file_data.private;

    let file_mime = match upload_file_data.file_mime_type.as_str() {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            upload_file_data.file_mime_type
        }
        _ => {
            return Err(ServiceError::BadRequest(
                "Must upload a docx file".to_string(),
            ))?;
        }
    };

    let conversion_result = convert_docx_to_html_query(
        upload_file_data.file_name,
        decoded_file_data,
        file_mime,
        private,
        user,
        pool_inner,
    )
    .await
    .map_err(|e| ServiceError::BadRequest(e.message.to_string()))?;

    Ok(HttpResponse::Ok().json(conversion_result))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpdateFileData {
    pub file_id: uuid::Uuid,
    pub private: bool,
}

pub async fn update_file_handler(
    data: web::Json<UpdateFileData>,
    pool: web::Data<Pool>,
    user: LoggedUser,
) -> Result<HttpResponse, actix_web::Error> {
    let thread_safe_pool = Arc::new(Mutex::new(pool));

    let pool_inner = thread_safe_pool.clone();
    user_owns_file(user.id, data.file_id, thread_safe_pool).await?;

    web::block(move || update_file_query(data.file_id, data.private, pool_inner.lock().unwrap()))
        .await?
        .map_err(|e| ServiceError::BadRequest(e.message.to_string()))?;

    Ok(HttpResponse::NoContent().finish())
}

pub async fn get_file_handler(
    file_id: web::Path<uuid::Uuid>,
    pool: web::Data<Pool>,
    user: Option<LoggedUser>,
) -> Result<HttpResponse, actix_web::Error> {
    let user_id = user.map(|user| user.id);

    let file = get_file_query(file_id.into_inner(), user_id, pool).await?;

    Ok(HttpResponse::Ok().json(file))
}

pub async fn get_user_files_handler(
    user_id: web::Path<uuid::Uuid>,
    pool: web::Data<Pool>,
    user: Option<LoggedUser>,
) -> Result<HttpResponse, actix_web::Error> {
    let accessing_user_id = user.map(|u| u.id);
    let user_id = user_id.into_inner();

    let files = get_user_file_query(user_id, accessing_user_id, pool).await?;

    Ok(HttpResponse::Ok().json(files))
}

pub async fn delete_file_handler(
    file_id: web::Path<uuid::Uuid>,
    pool: web::Data<Pool>,
    user: LoggedUser,
) -> Result<HttpResponse, actix_web::Error> {
    delete_file_query(file_id.into_inner(), user.id, pool).await?;

    Ok(HttpResponse::NoContent().finish())
}
