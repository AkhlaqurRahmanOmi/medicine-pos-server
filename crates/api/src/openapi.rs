use utoipa::OpenApi;

use crate::health;

#[derive(OpenApi)]
#[openapi(
    paths(health::live, health::ready,),
    info(
        title = "Medicine POS API",
        version = "1.0.0",
        description = "Pharmacy POS Backend API"
    )
)]
pub struct ApiDoc;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_document_builds() {
        let doc = ApiDoc::openapi();
        assert_eq!(doc.info.title, "Medicine POS API");
        assert!(!doc.paths.paths.is_empty());
        let json_str = doc.to_json().expect("serialize openapi doc");
        assert!(json_str.contains("/health"));
    }
}
