use pos_core::error::AppError;
use sea_orm::DbErr;

#[must_use]
pub fn db_error_to_app_error(err: DbErr) -> AppError {
    match &err {
        DbErr::RecordNotFound(msg) => AppError::not_found(msg),
        DbErr::Custom(msg) => {
            if msg.contains("duplicate key") || msg.contains("violates unique constraint") {
                AppError::conflict(msg)
            } else if msg.contains("violates row-level security policy")
                || msg.contains("permission denied")
            {
                AppError::forbidden(msg)
            } else {
                AppError::internal("Database error", msg)
            }
        }
        DbErr::Query(runtime_err) => {
            let msg = runtime_err.to_string();
            if msg.contains("violates unique constraint") || msg.contains("duplicate key") {
                AppError::conflict("Resource conflict: unique constraint violation")
            } else if msg.contains("violates row-level security policy")
                || msg.contains("permission denied")
            {
                AppError::forbidden("Access forbidden: row-level security policy violation")
            } else if msg.contains("violates foreign key constraint") {
                AppError::conflict("Resource conflict: foreign key constraint violation")
            } else {
                AppError::internal("Database query error", msg)
            }
        }
        _ => AppError::internal("Database error", err.to_string()),
    }
}

pub trait MapDbErr<T> {
    fn map_app_err(self) -> Result<T, AppError>;
}

impl<T> MapDbErr<T> for Result<T, DbErr> {
    fn map_app_err(self) -> Result<T, AppError> {
        self.map_err(db_error_to_app_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pos_core::error::ErrorCode;

    #[test]
    fn db_error_maps_to_app_error() {
        let not_found_err = DbErr::RecordNotFound("Item 123".into());
        let app_err = db_error_to_app_error(not_found_err);
        assert_eq!(app_err.code, ErrorCode::NotFound);

        let conflict_err = DbErr::Custom("duplicate key value violates unique constraint".into());
        let app_err = db_error_to_app_error(conflict_err);
        assert_eq!(app_err.code, ErrorCode::Conflict(ErrorCode::GENERIC_CONFLICT));

        let forbidden_err =
            DbErr::Custom("new row violates row-level security policy for table".into());
        let app_err = db_error_to_app_error(forbidden_err);
        assert_eq!(app_err.code, ErrorCode::PermissionDenied);

        let internal_err = DbErr::Custom("connection pool timeout".into());
        let app_err = db_error_to_app_error(internal_err);
        assert_eq!(app_err.code, ErrorCode::Internal);
    }
}
