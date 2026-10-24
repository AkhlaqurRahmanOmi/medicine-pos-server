use crate::types::{
    Company, Generic, IssueReport, NewCompany, NewGeneric, NewIssueReport, NewProduct, Product,
};
use db::Tx;
use pos_core::error::AppError;

pub async fn submit_company(tx: &Tx, input: NewCompany) -> Result<Company, AppError> {
    crate::repo::submit_company(tx, input).await
}

pub async fn submit_generic(tx: &Tx, input: NewGeneric) -> Result<Generic, AppError> {
    crate::repo::submit_generic(tx, input).await
}

pub async fn submit_product(tx: &Tx, input: NewProduct) -> Result<Product, AppError> {
    crate::repo::submit_product(tx, input).await
}

pub async fn report_issue(tx: &Tx, input: NewIssueReport) -> Result<IssueReport, AppError> {
    crate::repo::report_issue(tx, input).await
}
