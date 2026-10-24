use crate::types::{
    GenericId, IssueReport, IssueReportId, Monograph, MonographInput, RecordKind, Reviewed,
};
use db::Tx;
use pos_core::error::AppError;
use pos_core::ids::UserId;
use uuid::Uuid;

pub async fn approve(tx: &Tx, kind: RecordKind, id: Uuid, by: UserId) -> Result<Reviewed, AppError> {
    crate::repo::approve(tx, kind, id, by).await
}

pub async fn reject(
    tx: &Tx,
    kind: RecordKind,
    id: Uuid,
    by: UserId,
    note: String,
) -> Result<Reviewed, AppError> {
    crate::repo::reject(tx, kind, id, by, note).await
}

pub async fn merge(
    tx: &Tx,
    kind: RecordKind,
    id: Uuid,
    into: Uuid,
    by: UserId,
) -> Result<Reviewed, AppError> {
    crate::repo::merge(tx, kind, id, into, by).await
}

pub async fn upsert_monograph(
    tx: &Tx,
    id: GenericId,
    input: MonographInput,
) -> Result<Monograph, AppError> {
    crate::repo::upsert_monograph(tx, id, input).await
}

pub async fn resolve_issue(
    tx: &Tx,
    id: IssueReportId,
    by: UserId,
    note: String,
) -> Result<IssueReport, AppError> {
    crate::repo::resolve_issue(tx, id, by, note).await
}

pub async fn dismiss_issue(
    tx: &Tx,
    id: IssueReportId,
    by: UserId,
    note: String,
) -> Result<IssueReport, AppError> {
    crate::repo::dismiss_issue(tx, id, by, note).await
}
