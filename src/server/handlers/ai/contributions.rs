//! Contribution handlers for AI endpoints
//!
//! Provides endpoints for submitting, listing, and managing AI contributions.

use axum::{
    extract::{Path, Query, State},
    response::Json,
    Extension,
};
use serde::Deserialize;

use crate::server::auth::Claims;
use crate::server::handlers::AppState;
use crate::ai::{
    Contribution, ContributionStatus, ListContributionsResponse,
    ReviewContributionRequest, SubmitContributionRequest, SubmitContributionResponse,
};
use crate::error::DbError;

/// Query parameters for listing contributions
#[derive(Debug, Deserialize)]
pub struct ListContributionsQuery {
    /// Filter by status
    pub status: Option<String>,
    /// Filter by requester
    pub requester: Option<String>,
    /// Limit results
    pub limit: Option<usize>,
    /// Offset for pagination
    pub offset: Option<usize>,
}

/// POST /_api/ai/contributions - Submit a new contribution
///
/// Requires Write permission (any authenticated user with write access)
pub async fn submit_contribution_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Extension(claims): Extension<Claims>,
    Json(request): Json<SubmitContributionRequest>,
) -> Result<Json<SubmitContributionResponse>, DbError> {
    let username = claims.sub;
    let db = state.storage.get_database(&db_name)?;

    // Ensure _ai_contributions collection exists
    if db.get_collection("_ai_contributions").is_err() {
        db.create_collection("_ai_contributions".to_string(), None)?;
    }

    // Ensure _ai_tasks collection exists
    if db.get_collection("_ai_tasks").is_err() {
        db.create_collection("_ai_tasks".to_string(), None)?;
    }

    // Create the contribution using the constructor
    let contribution = Contribution::new(
        request.contribution_type,
        request.description,
        username,
        request.context,
    );

    let contribution_id = contribution.id.clone();

    // Store contribution in collection
    let coll = db.get_collection("_ai_contributions")?;
    let doc_value = serde_json::to_value(&contribution)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    coll.insert(doc_value)?;

    Ok(Json(SubmitContributionResponse {
        id: contribution_id,
        status: ContributionStatus::Submitted.to_string(),
        message: "Contribution submitted successfully".to_string(),
    }))
}

/// GET /_api/ai/contributions - List all contributions
///
/// Requires Read permission
pub async fn list_contributions_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Query(query): Query<ListContributionsQuery>,
) -> Result<Json<ListContributionsResponse>, DbError> {
    let db = state.storage.get_database(&db_name)?;

    // Return empty if collection doesn't exist
    if db.get_collection("_ai_contributions").is_err() {
        return Ok(Json(ListContributionsResponse {
            contributions: Vec::new(),
            total: 0,
        }));
    }

    let coll = db.get_collection("_ai_contributions")?;
    let mut contributions = Vec::new();

    for doc in coll.scan(None) {
        let contribution: Contribution = serde_json::from_value(doc.to_value())
            .map_err(|_| DbError::InternalError("Corrupted contribution data".to_string()))?;

        // Apply filters
        if let Some(ref status_filter) = query.status {
            let status_str = contribution.status.to_string();
            if status_str != *status_filter {
                continue;
            }
        }

        if let Some(ref requester_filter) = query.requester {
            if contribution.requester != *requester_filter {
                continue;
            }
        }

        contributions.push(contribution);
    }

    // Sort by context.priority descending, then by created_at ascending
    contributions.sort_by(|a, b| {
        b.context
            .priority
            .cmp(&a.context.priority)
            .then_with(|| a.created_at.cmp(&b.created_at))
    });

    let total = contributions.len();

    // Apply pagination
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(100);
    let contributions: Vec<Contribution> = contributions.into_iter().skip(offset).take(limit).collect();

    Ok(Json(ListContributionsResponse { contributions, total }))
}

/// GET /_api/ai/contributions/:id - Get a specific contribution
///
/// Requires Read permission
pub async fn get_contribution_handler(
    State(state): State<AppState>,
    Path((db_name, contribution_id)): Path<(String, String)>,
) -> Result<Json<Contribution>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_contributions")?;

    let doc = coll.get(&contribution_id)?;

    let contribution: Contribution = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted contribution data".to_string()))?;

    Ok(Json(contribution))
}

/// POST /_api/ai/contributions/:id/approve - Approve a contribution
///
/// Requires Admin permission.
pub async fn approve_contribution_handler(
    State(state): State<AppState>,
    Path((db_name, contribution_id)): Path<(String, String)>,
    Json(request): Json<ReviewContributionRequest>,
) -> Result<Json<Contribution>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_contributions")?;

    let doc = coll.get(&contribution_id)?;
    let mut contribution: Contribution = serde_json::from_value(doc.to_value())
        .map_err(|e| DbError::InternalError(format!("Corrupted contribution data: {}", e)))?;

    if contribution.status != ContributionStatus::Review {
        return Err(DbError::BadRequest(format!(
            "Contribution {} is not in review status (current status: {})",
            contribution_id,
            contribution.status
        )));
    }

    contribution.status = ContributionStatus::Approved;
    if let Some(ref feedback) = request.feedback {
        contribution.feedback = Some(format!("Approved by admin: {}", feedback));
    }
    contribution.updated_at = chrono::Utc::now();

    let doc_value = serde_json::to_value(&contribution)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    coll.update(&contribution_id, doc_value)?;

    Ok(Json(contribution))
}

/// POST /_api/ai/contributions/:id/reject - Reject a contribution
///
/// Requires Admin permission
pub async fn reject_contribution_handler(
    State(state): State<AppState>,
    Path((db_name, contribution_id)): Path<(String, String)>,
    Json(request): Json<ReviewContributionRequest>,
) -> Result<Json<Contribution>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_contributions")?;

    let doc = coll.get(&contribution_id)?;
    let mut contribution: Contribution = serde_json::from_value(doc.to_value())
        .map_err(|e| DbError::InternalError(format!("Corrupted contribution data: {}", e)))?;

    if contribution.status != ContributionStatus::Review {
        return Err(DbError::BadRequest(format!(
            "Contribution {} is not in review status (current status: {})",
            contribution_id,
            contribution.status
        )));
    }

    contribution.status = ContributionStatus::Rejected;
    if let Some(ref feedback) = request.feedback {
        contribution.feedback = Some(format!("Rejected by admin: {}", feedback));
    }
    contribution.updated_at = chrono::Utc::now();

    let doc_value = serde_json::to_value(&contribution)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    coll.update(&contribution_id, doc_value)?;

    Ok(Json(contribution))
}

/// POST /_api/ai/contributions/:id/cancel - Cancel a contribution (by requester)
///
/// Requires Write permission (only the requester can cancel)
pub async fn cancel_contribution_handler(
    State(state): State<AppState>,
    Path((db_name, contribution_id)): Path<(String, String)>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<Contribution>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_contributions")?;

    let doc = coll.get(&contribution_id)?;
    let mut contribution: Contribution = serde_json::from_value(doc.to_value())
        .map_err(|e| DbError::InternalError(format!("Corrupted contribution data: {}", e)))?;

    // Verify the requester is the one who submitted
    if contribution.requester != claims.sub {
        return Err(DbError::Forbidden(
            "You can only cancel your own contributions".to_string(),
        ));
    }

    match contribution.status {
        ContributionStatus::Submitted | ContributionStatus::Analyzing | ContributionStatus::Generating => {}
        _ => {
            return Err(DbError::BadRequest(format!(
                "Cannot cancel contribution in status {}",
                contribution.status
            )));
        }
    }

    contribution.status = ContributionStatus::Cancelled;
    contribution.updated_at = chrono::Utc::now();

    // Update in collection
    let doc_value = serde_json::to_value(&contribution)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    coll.update(&contribution_id, doc_value)?;

    Ok(Json(contribution))
}
