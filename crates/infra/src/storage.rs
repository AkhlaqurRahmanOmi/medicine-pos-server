use pos_core::error::{AppError, ErrorCode};
use pos_core::ports::FileStorage;

#[derive(Debug, Clone)]
pub struct S3StorageConfig {
    pub endpoint: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
}

#[derive(Debug, Clone)]
pub struct FileStorageService {
    config: S3StorageConfig,
}

impl FileStorageService {
    pub fn from_config(config: &S3StorageConfig) -> Result<Self, AppError> {
        if config.endpoint.trim().is_empty() {
            return Err(AppError::new(
                ErrorCode::Internal,
                "Storage endpoint cannot be empty",
            ));
        }
        if config.bucket.trim().is_empty() {
            return Err(AppError::new(
                ErrorCode::Internal,
                "Storage bucket cannot be empty",
            ));
        }
        Ok(Self {
            config: config.clone(),
        })
    }

    #[must_use]
    pub fn config(&self) -> &S3StorageConfig {
        &self.config
    }
}

impl FileStorage for FileStorageService {
    async fn put(&self, path: &str, _data: &[u8], _content_type: &str) -> Result<String, AppError> {
        Ok(format!("{}/{}", self.config.bucket, path))
    }

    async fn get(&self, _path: &str) -> Result<Vec<u8>, AppError> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_builds_from_config() {
        let config = S3StorageConfig {
            endpoint: "https://s3.ap-southeast-1.amazonaws.com".into(),
            bucket: "medicine-pos-slips".into(),
            access_key: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            region: "ap-southeast-1".into(),
        };

        let storage = FileStorageService::from_config(&config);
        assert!(
            storage.is_ok(),
            "FileStorageService must build from valid config"
        );
        assert_eq!(storage.unwrap().config().bucket, "medicine-pos-slips");
    }
}
