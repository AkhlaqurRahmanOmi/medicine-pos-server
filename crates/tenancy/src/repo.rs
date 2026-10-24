use crate::access::{self, AccessFacts, AccessMode, SubscriptionWindow, TenantStatus};
use crate::entity::{
    branches, plan_limit_types, plan_limits, plan_prices, plans, subscription_payments,
    subscriptions, tenant_exports, tenant_settings, tenants,
};
use crate::types::{
    AccessSummary, Branch, BranchPatch, NewBranch, NewPayment, NewPlanRequest, NewSubscription,
    Payment, Plan, PlanLimit, PlanPrice, SettingsPatch, Subscription, Tenant, TenantDetail,
    TenantExport, TenantPatch, TenantSettings, TenantSummary, UpdatePlanRequest,
};
use api::envelope::ListPage;
use api::extract::Pagination;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use db::{db_error_to_app_error, Db, Tx};
use pos_core::context::TenantContext;
use pos_core::error::{AppError, ErrorCode};
use pos_core::ids::{BranchId, TenantId, UserId};
use rust_decimal::Decimal;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use uuid::Uuid;

pub fn model_to_branch(m: branches::Model) -> Branch {
    Branch {
        id: BranchId::from_uuid(m.id),
        tenant_id: TenantId::from_uuid(m.tenant_id),
        name: m.name,
        address: m.address,
        phone: m.phone,
        drug_license_no: m.drug_license_no,
        is_default: m.is_default,
        is_active: m.is_active,
        created_at: m.created_at.with_timezone(&Utc),
    }
}

pub fn model_to_settings(m: tenant_settings::Model) -> TenantSettings {
    TenantSettings {
        vat_rate: m.vat_rate,
        vat_included: m.vat_included,
        rounding_mode: m.rounding_mode,
        near_expiry_days: m.near_expiry_days,
        default_payment_terms_days: m.default_payment_terms_days,
        allow_zero_stock_sale: m.allow_zero_stock_sale,
        receipt_header: m.receipt_header,
        receipt_footer: m.receipt_footer,
        receipt_paper_width: m.receipt_paper_width,
    }
}

pub fn model_to_payment(m: subscription_payments::Model) -> Payment {
    Payment {
        id: m.id,
        subscription_id: m.subscription_id,
        amount: m.amount,
        method: m.method,
        reference: m.reference,
        status: m.status,
        paid_at: m.paid_at.with_timezone(&Utc),
    }
}

pub fn access_mode_to_summary(mode: &AccessMode) -> AccessSummary {
    let (mode_str, warning_until) = match mode {
        AccessMode::Blocked(access::BlockReason::PendingApproval) => ("pending_approval", None),
        AccessMode::Blocked(access::BlockReason::Suspended) => ("suspended", None),
        AccessMode::ReadOnly => ("read_only", None),
        AccessMode::Full => ("full", None),
        AccessMode::FullWithWarning { until } => ("full_with_warning", Some(*until)),
    };
    AccessSummary {
        mode: mode_str.to_string(),
        warning_until,
    }
}

pub fn model_to_tenant(m: tenants::Model, access: AccessSummary) -> Tenant {
    Tenant {
        id: TenantId::from_uuid(m.id),
        name: m.name,
        legal_name: m.legal_name,
        vat_bin: m.vat_bin,
        phone: m.phone,
        email: m.email,
        status: m.status,
        access,
    }
}

pub async fn get_access_mode_with_conn<C: sea_orm::ConnectionTrait>(
    conn: &C,
    tenant: TenantId,
    now: DateTime<Utc>,
) -> Result<AccessMode, AppError> {
    let tenant_model = tenants::Entity::find_by_id(tenant.as_uuid())
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Tenant not found"))?;

    let status = TenantStatus::try_from(tenant_model.status.as_str())?;

    if matches!(
        status,
        TenantStatus::PendingApproval | TenantStatus::Suspended
    ) {
        return Ok(access::decide(
            &AccessFacts {
                status,
                covering: None,
                latest_end: None,
                grace_days: 0,
            },
            now,
        ));
    }

    let subs = subscriptions::Entity::find()
        .filter(subscriptions::Column::TenantId.eq(tenant.as_uuid()))
        .order_by_desc(subscriptions::Column::EndsAt)
        .limit(2)
        .all(conn)
        .await
        .map_err(db_error_to_app_error)?;

    let covering = subs.iter().find(|s| {
        s.status == "active"
            && s.starts_at.with_timezone(&Utc) <= now
            && now <= s.ends_at.with_timezone(&Utc)
    });

    let covering_window = covering.map(|s| SubscriptionWindow {
        starts_at: s.starts_at.with_timezone(&Utc),
        ends_at: s.ends_at.with_timezone(&Utc),
    });

    let latest_sub = subs.first();

    let (latest_end, grace_days) = if let Some(sub) = latest_sub {
        let plan = plans::Entity::find_by_id(sub.plan_id)
            .one(conn)
            .await
            .map_err(db_error_to_app_error)?;
        let grace = plan.map(|p| p.grace_days).unwrap_or(7);
        (Some(sub.ends_at.with_timezone(&Utc)), grace)
    } else {
        (None, 0)
    };

    let facts = AccessFacts {
        status,
        covering: covering_window,
        latest_end,
        grace_days,
    };

    Ok(access::decide(&facts, now))
}

pub async fn get_access_mode(
    db: &Db,
    tenant: TenantId,
    now: DateTime<Utc>,
) -> Result<AccessMode, AppError> {
    let ctx = TenantContext::new(tenant, UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await?;
    let mode = get_access_mode_with_conn(tx.transaction()?, tenant, now).await?;
    tx.commit().await?;
    Ok(mode)
}

pub async fn get_tenant(db: &Db, tenant: TenantId) -> Result<Tenant, AppError> {
    let ctx = TenantContext::new(tenant, UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await?;
    let conn = tx.transaction()?;
    let m = tenants::Entity::find_by_id(tenant.as_uuid())
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Tenant not found"))?;

    let mode = get_access_mode_with_conn(conn, tenant, Utc::now()).await?;
    let access = access_mode_to_summary(&mode);
    let result = model_to_tenant(m, access);
    tx.commit().await?;
    Ok(result)
}

pub async fn get_settings(db: &Db, tenant: TenantId) -> Result<TenantSettings, AppError> {
    let ctx = TenantContext::new(tenant, UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await?;
    let m = tenant_settings::Entity::find_by_id(tenant.as_uuid())
        .one(tx.transaction()?)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Tenant settings not found"))?;
    let result = model_to_settings(m);
    tx.commit().await?;
    Ok(result)
}

pub async fn get_branches(
    db: &Db,
    tenant: TenantId,
    page: Pagination,
) -> Result<ListPage<Branch>, AppError> {
    let ctx = TenantContext::new(tenant, UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await?;
    let res = get_branches_with_conn(tx.transaction()?, tenant, page).await?;
    tx.commit().await?;
    Ok(res)
}

pub async fn get_branches_with_conn<C: sea_orm::ConnectionTrait>(
    conn: &C,
    tenant: TenantId,
    page: Pagination,
) -> Result<ListPage<Branch>, AppError> {
    let mut query = branches::Entity::find()
        .filter(branches::Column::TenantId.eq(tenant.as_uuid()))
        .order_by_asc(branches::Column::Id);

    if let Some(ref cursor_str) = page.cursor {
        let cursor_uuid = Uuid::parse_str(cursor_str)
            .map_err(|_| AppError::bad_request("Invalid cursor format"))?;
        query = query.filter(branches::Column::Id.gt(cursor_uuid));
    }

    let limit_to_fetch = (page.limit as u64) + 1;
    let rows = query
        .limit(limit_to_fetch)
        .all(conn)
        .await
        .map_err(db_error_to_app_error)?;

    let has_more = rows.len() > page.limit as usize;
    let mut page_rows = rows;
    if has_more {
        page_rows.truncate(page.limit as usize);
    }

    let next_cursor = if has_more {
        page_rows.last().map(|r| r.id.to_string())
    } else {
        None
    };

    let data = page_rows.into_iter().map(model_to_branch).collect();
    Ok(ListPage::new(data, next_cursor))
}

pub async fn get_limit_remaining<C: sea_orm::ConnectionTrait>(
    conn: &C,
    tenant: TenantId,
    key: &str,
) -> Result<Option<i32>, AppError> {
    match key {
        "branches" | "users" | "devices" => {}
        other => {
            return Err(AppError::bad_request(format!(
                "Unimplemented limit type '{other}'"
            )));
        }
    }

    let now = Utc::now();
    let sub = subscriptions::Entity::find()
        .filter(subscriptions::Column::TenantId.eq(tenant.as_uuid()))
        .filter(subscriptions::Column::Status.eq("active"))
        .filter(subscriptions::Column::StartsAt.lte(now))
        .filter(subscriptions::Column::EndsAt.gte(now))
        .order_by_desc(subscriptions::Column::EndsAt)
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?;

    let plan_id = if let Some(s) = sub {
        s.plan_id
    } else {
        let fallback_sub = subscriptions::Entity::find()
            .filter(subscriptions::Column::TenantId.eq(tenant.as_uuid()))
            .order_by_desc(subscriptions::Column::EndsAt)
            .one(conn)
            .await
            .map_err(db_error_to_app_error)?;

        if let Some(s) = fallback_sub {
            s.plan_id
        } else {
            let standard_plan = plans::Entity::find()
                .filter(plans::Column::Code.eq("standard"))
                .one(conn)
                .await
                .map_err(db_error_to_app_error)?;
            match standard_plan {
                Some(p) => p.id,
                None => return Ok(None),
            }
        }
    };

    let limit_row = plan_limits::Entity::find_by_id((plan_id, key.to_string()))
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?;

    let max_val = match limit_row {
        Some(l) => match l.max_value {
            Some(v) => v,
            None => return Ok(None),
        },
        None => return Ok(None),
    };

    let current_count: i64 = match key {
        "branches" => {
            branches::Entity::find()
                .filter(branches::Column::TenantId.eq(tenant.as_uuid()))
                .filter(branches::Column::IsActive.eq(true))
                .count(conn)
                .await
                .map_err(db_error_to_app_error)? as i64
        }
        "users" => {
            return Err(AppError::bad_request(
                "Limit checking for users is not implemented in tenancy module",
            ));
        }
        "devices" => {
            return Err(AppError::bad_request(
                "Limit checking for devices is not implemented in tenancy module",
            ));
        }
        other => {
            return Err(AppError::bad_request(format!(
                "Unimplemented limit type '{other}'"
            )));
        }
    };

    let count_i32 = i32::try_from(current_count).unwrap_or(i32::MAX);
    let remaining = max_val - count_i32;
    Ok(Some(remaining.max(0)))
}

pub async fn get_limit_remaining_query(
    db: &Db,
    tenant: TenantId,
    key: &str,
) -> Result<Option<i32>, AppError> {
    let ctx = TenantContext::new(tenant, UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await?;
    let rem = get_limit_remaining(tx.transaction()?, tenant, key).await?;
    tx.commit().await?;
    Ok(rem)
}

pub async fn get_plans(db: &Db) -> Result<Vec<Plan>, AppError> {
    let all_plans = plans::Entity::find()
        .order_by_asc(plans::Column::Code)
        .all(db.conn())
        .await
        .map_err(db_error_to_app_error)?;

    let all_prices = plan_prices::Entity::find()
        .order_by_asc(plan_prices::Column::BillingPeriod)
        .all(db.conn())
        .await
        .map_err(db_error_to_app_error)?;

    let all_limits = plan_limits::Entity::find()
        .order_by_asc(plan_limits::Column::LimitKey)
        .all(db.conn())
        .await
        .map_err(db_error_to_app_error)?;

    let mut prices_by_plan: std::collections::HashMap<Uuid, Vec<PlanPrice>> =
        std::collections::HashMap::new();
    for pr in all_prices {
        prices_by_plan
            .entry(pr.plan_id)
            .or_default()
            .push(PlanPrice {
                billing_period: pr.billing_period,
                price: pr.price,
            });
    }

    let mut limits_by_plan: std::collections::HashMap<Uuid, Vec<PlanLimit>> =
        std::collections::HashMap::new();
    for li in all_limits {
        limits_by_plan
            .entry(li.plan_id)
            .or_default()
            .push(PlanLimit {
                limit_key: li.limit_key,
                max_value: li.max_value,
            });
    }

    let result = all_plans
        .into_iter()
        .map(|p| {
            let prices = prices_by_plan.remove(&p.id).unwrap_or_default();
            let limits = limits_by_plan.remove(&p.id).unwrap_or_default();
            Plan {
                id: p.id,
                code: p.code,
                name: p.name,
                grace_days: p.grace_days,
                is_active: p.is_active,
                prices,
                limits,
            }
        })
        .collect();

    Ok(result)
}

pub async fn create_pharmacy(
    tx: &Tx,
    name: &str,
    legal_name: Option<String>,
    vat_bin: Option<String>,
    phone: Option<String>,
    email: Option<String>,
) -> Result<Tenant, AppError> {
    let conn = tx.transaction()?;
    let tenant_id = tx
        .tenant_id()
        .unwrap_or_else(|_| TenantId::new_v7());
    let now = Utc::now();

    let tenant_active = tenants::ActiveModel {
        id: Set(tenant_id.as_uuid()),
        name: Set(name.to_string()),
        legal_name: Set(legal_name),
        vat_bin: Set(vat_bin),
        phone: Set(phone),
        email: Set(email),
        status: Set("pending_approval".to_string()),
        suspension_reason: Set(None),
        approved_by_platform_user_id: Set(None),
        approved_at: Set(None),
        deletion_requested_at: Set(None),
        purge_at: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    };
    let tenant_model = tenant_active.insert(conn).await.map_err(|e| {
        let is_conflict = matches!(e.sql_err(), Some(sea_orm::SqlErr::UniqueConstraintViolation(_)))
            || e.to_string().contains("violates unique constraint");
        if is_conflict {
            AppError::conflict("Tenant already exists")
        } else {
            db_error_to_app_error(e)
        }
    })?;

    let branch_active = branches::ActiveModel {
        id: Set(BranchId::new_v7().as_uuid()),
        tenant_id: Set(tenant_id.as_uuid()),
        name: Set("Main".to_string()),
        address: Set(None),
        phone: Set(None),
        drug_license_no: Set(None),
        is_default: Set(true),
        is_active: Set(true),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    };
    branch_active
        .insert(conn)
        .await
        .map_err(db_error_to_app_error)?;

    let settings_active = tenant_settings::ActiveModel {
        tenant_id: Set(tenant_id.as_uuid()),
        vat_rate: Set(Decimal::ZERO),
        vat_included: Set(false),
        rounding_mode: Set("nearest_half".to_string()),
        near_expiry_days: Set(90),
        default_payment_terms_days: Set(30),
        allow_zero_stock_sale: Set(false),
        receipt_header: Set(None),
        receipt_footer: Set(None),
        receipt_paper_width: Set(80),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    };
    settings_active
        .insert(conn)
        .await
        .map_err(db_error_to_app_error)?;

    Ok(Tenant {
        id: TenantId::from_uuid(tenant_model.id),
        name: tenant_model.name,
        legal_name: tenant_model.legal_name,
        vat_bin: tenant_model.vat_bin,
        phone: tenant_model.phone,
        email: tenant_model.email,
        status: tenant_model.status,
        access: AccessSummary {
            mode: "pending_approval".to_string(),
            warning_until: None,
        },
    })
}

pub async fn create_branch(tx: &Tx, input: NewBranch) -> Result<Branch, AppError> {
    let conn = tx.transaction()?;
    let tenant_id = tx.tenant_id()?;

    // C6: Acquire exclusive lock on the tenant row before checking limits
    // to serialize branch creations for this pharmacy under concurrency.
    tenants::Entity::find_by_id(tenant_id.as_uuid())
        .lock_exclusive()
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?;

    let remaining = get_limit_remaining(conn, tenant_id, "branches").await?;
    if let Some(rem) = remaining {
        if rem <= 0 {
            return Err(AppError::conflict_code(
                "plan_limit_reached",
                "Branch limit reached for current plan",
            ));
        }
    }

    let now = Utc::now();
    let branch_active = branches::ActiveModel {
        id: Set(BranchId::new_v7().as_uuid()),
        tenant_id: Set(tenant_id.as_uuid()),
        name: Set(input.name),
        address: Set(input.address),
        phone: Set(input.phone),
        drug_license_no: Set(input.drug_license_no),
        is_default: Set(false),
        is_active: Set(true),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    };

    let model = branch_active.insert(conn).await.map_err(|e| {
        let is_name_conflict = matches!(e.sql_err(), Some(sea_orm::SqlErr::UniqueConstraintViolation(ref s)) if s.contains("uq_branches_tenant_id_name") || s.contains("name"))
            || e.to_string().contains("uq_branches_tenant_id_name");
        if is_name_conflict {
            AppError::conflict_code(
                "duplicate_branch_name",
                "A branch with this name already exists in this pharmacy",
            )
        } else {
            db_error_to_app_error(e)
        }
    })?;

    Ok(model_to_branch(model))
}

pub async fn deactivate_branch(tx: &Tx, id: BranchId) -> Result<Branch, AppError> {
    let conn = tx.transaction()?;
    let tenant_id = tx.tenant_id()?;

    let branch = branches::Entity::find_by_id(id.as_uuid())
        .filter(branches::Column::TenantId.eq(tenant_id.as_uuid()))
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Branch not found"))?;

    if branch.is_default {
        return Err(AppError::conflict_code(
            "last_active_branch",
            "The default branch cannot be deactivated",
        ));
    }

    let active_count = branches::Entity::find()
        .filter(branches::Column::TenantId.eq(tenant_id.as_uuid()))
        .filter(branches::Column::IsActive.eq(true))
        .count(conn)
        .await
        .map_err(db_error_to_app_error)?;

    if active_count <= 1 {
        return Err(AppError::conflict_code(
            "last_active_branch",
            "The last active branch cannot be deactivated",
        ));
    }

    let mut active: branches::ActiveModel = branch.into();
    active.is_active = Set(false);
    active.updated_at = Set(Utc::now().into());

    let updated = active.update(conn).await.map_err(db_error_to_app_error)?;
    Ok(model_to_branch(updated))
}

pub async fn update_settings(
    tx: &Tx,
    input: SettingsPatch,
) -> Result<TenantSettings, AppError> {
    let conn = tx.transaction()?;
    let tenant_id = tx.tenant_id()?;

    if let Some(vat_rate) = input.vat_rate {
        if vat_rate < Decimal::ZERO || vat_rate > Decimal::from(100) {
            return Err(AppError::new(
                ErrorCode::ValidationFailed,
                "VAT rate must be between 0.00 and 100.00",
            )
            .detail_field("vat_rate", "must be between 0.00 and 100.00"));
        }
    }

    let settings = tenant_settings::Entity::find_by_id(tenant_id.as_uuid())
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Tenant settings not found"))?;

    let mut active: tenant_settings::ActiveModel = settings.into();

    if let Some(vat_rate) = input.vat_rate {
        active.vat_rate = Set(vat_rate);
    }
    if let Some(vat_included) = input.vat_included {
        active.vat_included = Set(vat_included);
    }
    if let Some(rounding_mode) = input.rounding_mode {
        active.rounding_mode = Set(rounding_mode);
    }
    if let Some(near_expiry_days) = input.near_expiry_days {
        active.near_expiry_days = Set(near_expiry_days);
    }
    if let Some(default_payment_terms_days) = input.default_payment_terms_days {
        active.default_payment_terms_days = Set(default_payment_terms_days);
    }
    if let Some(allow_zero_stock_sale) = input.allow_zero_stock_sale {
        active.allow_zero_stock_sale = Set(allow_zero_stock_sale);
    }
    if let Some(receipt_header) = input.receipt_header {
        active.receipt_header = Set(receipt_header);
    }
    if let Some(receipt_footer) = input.receipt_footer {
        active.receipt_footer = Set(receipt_footer);
    }
    if let Some(receipt_paper_width) = input.receipt_paper_width {
        active.receipt_paper_width = Set(receipt_paper_width);
    }
    active.updated_at = Set(Utc::now().into());

    let updated = active.update(conn).await.map_err(db_error_to_app_error)?;
    Ok(model_to_settings(updated))
}

pub async fn submit_payment(tx: &Tx, input: NewPayment) -> Result<Payment, AppError> {
    let conn = tx.transaction()?;
    let tenant_id = tx.tenant_id()?;
    let now = Utc::now();

    let payment_active = subscription_payments::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(Some(tenant_id.as_uuid())),
        subscription_id: Set(input.subscription_id),
        amount: Set(input.amount),
        method: Set(input.method),
        reference: Set(input.reference),
        proof_file_key: Set(input.proof_file_key),
        status: Set("submitted".to_string()),
        review_note: Set(None),
        reviewed_by_platform_user_id: Set(None),
        reviewed_at: Set(None),
        paid_at: Set(now.into()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    };

    let model = payment_active.insert(conn).await.map_err(|e| {
        let is_ref_conflict = matches!(e.sql_err(), Some(sea_orm::SqlErr::UniqueConstraintViolation(ref s)) if s.contains("idx_subscription_payments_method_ref") || s.contains("reference"))
            || e.to_string()
                .contains("idx_subscription_payments_method_ref");
        if is_ref_conflict {
            AppError::conflict_code(
                "duplicate_payment_reference",
                "Payment reference already exists for this payment method",
            )
        } else {
            db_error_to_app_error(e)
        }
    })?;

    Ok(model_to_payment(model))
}

pub async fn request_closure(tx: &Tx, _by: UserId) -> Result<Tenant, AppError> {
    let conn = tx.transaction()?;
    let tenant_id = tx.tenant_id()?;

    let tenant = tenants::Entity::find_by_id(tenant_id.as_uuid())
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Tenant not found"))?;

    if tenant.status == "closing" {
        return Err(AppError::conflict_code(
            "already_closing",
            "Tenant is already in closing state",
        ));
    }

    let now = Utc::now();
    let purge_at = now + ChronoDuration::days(30);

    let mut active: tenants::ActiveModel = tenant.into();
    active.status = Set("closing".to_string());
    active.deletion_requested_at = Set(Some(now.into()));
    active.purge_at = Set(Some(purge_at.into()));
    active.updated_at = Set(now.into());

    let updated = active.update(conn).await.map_err(db_error_to_app_error)?;

    let mode = get_access_mode_with_conn(conn, tenant_id, now).await?;
    let access = access_mode_to_summary(&mode);
    Ok(model_to_tenant(updated, access))
}

pub async fn approve_tenant(
    tx: &Tx,
    tenant_id: TenantId,
    by: UserId,
) -> Result<Tenant, AppError> {
    let conn = tx.transaction()?;
    let tenant = tenants::Entity::find_by_id(tenant_id.as_uuid())
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Tenant not found"))?;

    if tenant.status != "pending_approval" {
        return Err(AppError::conflict_code(
            "not_pending_approval",
            "Tenant is not pending approval",
        ));
    }

    let now = Utc::now();
    let mut active: tenants::ActiveModel = tenant.into();
    active.status = Set("active".to_string());
    active.approved_by_platform_user_id = Set(Some(by.as_uuid()));
    active.approved_at = Set(Some(now.into()));
    active.updated_at = Set(now.into());

    let updated = active.update(conn).await.map_err(db_error_to_app_error)?;

    let mode = get_access_mode_with_conn(conn, tenant_id, now).await?;
    let access = access_mode_to_summary(&mode);
    Ok(model_to_tenant(updated, access))
}

pub async fn suspend_tenant(
    tx: &Tx,
    tenant_id: TenantId,
    reason: String,
) -> Result<Tenant, AppError> {
    if reason.trim().is_empty() {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            "Suspension requires a reason",
        )
        .detail_field("suspension_reason", "reason is required"));
    }

    let conn = tx.transaction()?;
    let tenant = tenants::Entity::find_by_id(tenant_id.as_uuid())
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Tenant not found"))?;

    if tenant.status == "suspended" {
        return Err(AppError::conflict_code(
            "already_suspended",
            "Tenant is already suspended",
        ));
    }

    let now = Utc::now();
    let mut active: tenants::ActiveModel = tenant.into();
    active.status = Set("suspended".to_string());
    active.suspension_reason = Set(Some(reason));
    active.updated_at = Set(now.into());

    let updated = active.update(conn).await.map_err(db_error_to_app_error)?;
    let mode = get_access_mode_with_conn(conn, tenant_id, now).await?;
    let access = access_mode_to_summary(&mode);
    Ok(model_to_tenant(updated, access))
}

pub async fn reinstate_tenant(tx: &Tx, tenant_id: TenantId) -> Result<Tenant, AppError> {
    let conn = tx.transaction()?;
    let tenant = tenants::Entity::find_by_id(tenant_id.as_uuid())
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Tenant not found"))?;

    if tenant.status != "suspended" {
        return Err(AppError::conflict_code(
            "not_suspended",
            "Tenant is not suspended",
        ));
    }

    let now = Utc::now();
    let mut active: tenants::ActiveModel = tenant.into();
    active.status = Set("active".to_string());
    active.suspension_reason = Set(None);
    active.updated_at = Set(now.into());

    let updated = active.update(conn).await.map_err(db_error_to_app_error)?;
    let mode = get_access_mode_with_conn(conn, tenant_id, now).await?;
    let access = access_mode_to_summary(&mode);
    Ok(model_to_tenant(updated, access))
}

pub async fn confirm_payment(
    tx: &Tx,
    payment_id: Uuid,
) -> Result<(Payment, Subscription), AppError> {
    let conn = tx.transaction()?;
    let reviewer = tx
        .user_id()
        .unwrap_or_else(|_| UserId::from_uuid(Uuid::nil()));

    let payment = subscription_payments::Entity::find_by_id(payment_id)
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Payment not found"))?;

    if payment.status != "submitted" {
        return Err(AppError::conflict_code(
            "already_reviewed",
            "Payment has already been reviewed",
        ));
    }

    let sub_id = payment
        .subscription_id
        .ok_or_else(|| AppError::bad_request("Payment is not linked to a subscription"))?;

    let sub = subscriptions::Entity::find_by_id(sub_id)
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Subscription not found"))?;

    let plan = plans::Entity::find_by_id(sub.plan_id)
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Plan not found"))?;

    let now = Utc::now();
    let mut sub_active: subscriptions::ActiveModel = sub.into();
    sub_active.status = Set("active".to_string());
    sub_active.updated_at = Set(now.into());
    let updated_sub = sub_active.update(conn).await.map_err(db_error_to_app_error)?;

    let mut pay_active: subscription_payments::ActiveModel = payment.into();
    pay_active.status = Set("confirmed".to_string());
    pay_active.reviewed_by_platform_user_id = Set(Some(reviewer.as_uuid()));
    pay_active.reviewed_at = Set(Some(now.into()));
    pay_active.updated_at = Set(now.into());
    let updated_pay = pay_active.update(conn).await.map_err(db_error_to_app_error)?;

    let sub_dto = Subscription {
        id: updated_sub.id,
        tenant_id: TenantId::from_uuid(updated_sub.tenant_id),
        plan_code: plan.code,
        billing_period: updated_sub.billing_period,
        price: updated_sub.price,
        starts_at: updated_sub.starts_at.with_timezone(&Utc),
        ends_at: updated_sub.ends_at.with_timezone(&Utc),
        status: updated_sub.status,
        is_complimentary: updated_sub.is_complimentary,
    };

    Ok((model_to_payment(updated_pay), sub_dto))
}

pub async fn reject_payment(
    tx: &Tx,
    payment_id: Uuid,
    note: String,
) -> Result<Payment, AppError> {
    if note.trim().is_empty() {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            "Rejecting a payment requires a review note",
        )
        .detail_field("review_note", "note is required"));
    }

    let conn = tx.transaction()?;
    let reviewer = tx
        .user_id()
        .unwrap_or_else(|_| UserId::from_uuid(Uuid::nil()));

    let payment = subscription_payments::Entity::find_by_id(payment_id)
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Payment not found"))?;

    if payment.status != "submitted" {
        return Err(AppError::conflict_code(
            "already_reviewed",
            "Payment has already been reviewed",
        ));
    }

    let now = Utc::now();
    let mut pay_active: subscription_payments::ActiveModel = payment.into();
    pay_active.status = Set("rejected".to_string());
    pay_active.review_note = Set(Some(note));
    pay_active.reviewed_by_platform_user_id = Set(Some(reviewer.as_uuid()));
    pay_active.reviewed_at = Set(Some(now.into()));
    pay_active.updated_at = Set(now.into());

    let updated = pay_active.update(conn).await.map_err(db_error_to_app_error)?;
    Ok(model_to_payment(updated))
}

pub async fn create_subscription(
    tx: &Tx,
    tenant_id: TenantId,
    input: NewSubscription,
) -> Result<Subscription, AppError> {
    if input.is_complimentary
        && input
            .grant_reason
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            "Complimentary subscription requires a grant_reason",
        )
        .detail_field("grant_reason", "reason is required"));
    }

    let conn = tx.transaction()?;

    let plan = plans::Entity::find_by_id(input.plan_id)
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Plan not found"))?;

    let plan_price = plan_prices::Entity::find_by_id((input.plan_id, input.billing_period.clone()))
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| {
            AppError::not_found("Plan price not found for this billing period")
        })?;

    let now = Utc::now();
    let sub_active = subscriptions::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant_id.as_uuid()),
        plan_id: Set(input.plan_id),
        billing_period: Set(input.billing_period),
        price: Set(plan_price.price),
        starts_at: Set(input.starts_at.into()),
        ends_at: Set(input.ends_at.into()),
        status: Set("active".to_string()),
        is_complimentary: Set(input.is_complimentary),
        grant_reason: Set(input.grant_reason),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    };

    let model = sub_active.insert(conn).await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("ex_subscriptions_no_overlap")
            || msg.contains("conflicting key value violates exclusion constraint")
        {
            AppError::conflict_code(
                "subscription_overlap",
                "Subscription overlaps with an existing subscription for this pharmacy",
            )
        } else if msg.contains("chk_subscriptions_complimentary_grant_reason") {
            AppError::validation_failed("Complimentary subscription requires a grant reason")
        } else {
            db_error_to_app_error(e)
        }
    })?;

    Ok(Subscription {
        id: model.id,
        tenant_id: TenantId::from_uuid(model.tenant_id),
        plan_code: plan.code,
        billing_period: model.billing_period,
        price: model.price,
        starts_at: model.starts_at.with_timezone(&Utc),
        ends_at: model.ends_at.with_timezone(&Utc),
        status: model.status,
        is_complimentary: model.is_complimentary,
    })
}

pub async fn payments_for_review(
    db: &Db,
    status: &str,
    page: Pagination,
) -> Result<ListPage<Payment>, AppError> {
    let ctx = TenantContext::new(TenantId::from_uuid(Uuid::nil()), UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin_platform(db.conn(), &ctx).await?;
    let res = payments_for_review_with_conn(tx.transaction()?, status, page).await?;
    tx.commit().await?;
    Ok(res)
}

pub async fn payments_for_review_with_conn<C: sea_orm::ConnectionTrait>(
    conn: &C,
    status: &str,
    page: Pagination,
) -> Result<ListPage<Payment>, AppError> {
    let mut query = subscription_payments::Entity::find()
        .filter(subscription_payments::Column::Status.eq(status))
        .order_by_asc(subscription_payments::Column::Id);

    if let Some(ref cursor_str) = page.cursor {
        let cursor_uuid = Uuid::parse_str(cursor_str)
            .map_err(|_| AppError::bad_request("Invalid cursor format"))?;
        query = query.filter(subscription_payments::Column::Id.gt(cursor_uuid));
    }

    let limit_to_fetch = (page.limit as u64) + 1;
    let rows = query
        .limit(limit_to_fetch)
        .all(conn)
        .await
        .map_err(db_error_to_app_error)?;

    let has_more = rows.len() > page.limit as usize;
    let mut page_rows = rows;
    if has_more {
        page_rows.truncate(page.limit as usize);
    }

    let next_cursor = if has_more {
        page_rows.last().map(|r| r.id.to_string())
    } else {
        None
    };

    let data = page_rows.into_iter().map(model_to_payment).collect();
    Ok(ListPage::new(data, next_cursor))
}

pub async fn update_tenant(tx: &Tx, patch: TenantPatch) -> Result<Tenant, AppError> {
    let conn = tx.transaction()?;
    let tenant_id = tx.tenant_id()?;

    let tenant = tenants::Entity::find_by_id(tenant_id.as_uuid())
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Tenant not found"))?;

    let mut active: tenants::ActiveModel = tenant.into();
    if let Some(name) = patch.name {
        if name.trim().is_empty() {
            return Err(AppError::validation_failed("name cannot be empty"));
        }
        active.name = Set(name);
    }
    if let Some(legal_name) = patch.legal_name {
        active.legal_name = Set(legal_name);
    }
    if let Some(vat_bin) = patch.vat_bin {
        active.vat_bin = Set(vat_bin);
    }
    if let Some(phone) = patch.phone {
        active.phone = Set(phone);
    }
    if let Some(email) = patch.email {
        active.email = Set(email);
    }
    active.updated_at = Set(Utc::now().into());

    let updated = active.update(conn).await.map_err(db_error_to_app_error)?;
    let mode = get_access_mode_with_conn(conn, tenant_id, Utc::now()).await?;
    let access = access_mode_to_summary(&mode);
    Ok(model_to_tenant(updated, access))
}

pub async fn get_branch_with_conn<C: sea_orm::ConnectionTrait>(
    conn: &C,
    tenant_id: TenantId,
    branch_id: BranchId,
) -> Result<Branch, AppError> {
    let b = branches::Entity::find_by_id(branch_id.as_uuid())
        .filter(branches::Column::TenantId.eq(tenant_id.as_uuid()))
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Branch not found"))?;
    Ok(model_to_branch(b))
}

pub async fn get_branch(
    db: &Db,
    tenant_id: TenantId,
    branch_id: BranchId,
) -> Result<Branch, AppError> {
    let ctx = TenantContext::new(tenant_id, UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await?;
    let b = get_branch_with_conn(tx.transaction()?, tenant_id, branch_id).await?;
    tx.commit().await?;
    Ok(b)
}

pub async fn update_branch(tx: &Tx, id: BranchId, patch: BranchPatch) -> Result<Branch, AppError> {
    let conn = tx.transaction()?;
    let tenant_id = tx.tenant_id()?;

    let branch = branches::Entity::find_by_id(id.as_uuid())
        .filter(branches::Column::TenantId.eq(tenant_id.as_uuid()))
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Branch not found"))?;

    let mut active: branches::ActiveModel = branch.into();
    if let Some(name) = patch.name {
        if name.trim().is_empty() {
            return Err(AppError::validation_failed("name cannot be empty"));
        }
        active.name = Set(name);
    }
    if let Some(address) = patch.address {
        active.address = Set(address);
    }
    if let Some(phone) = patch.phone {
        active.phone = Set(phone);
    }
    if let Some(drug_license_no) = patch.drug_license_no {
        active.drug_license_no = Set(drug_license_no);
    }
    active.updated_at = Set(Utc::now().into());

    let updated = active.update(conn).await.map_err(|e| {
        let is_name_conflict = matches!(e.sql_err(), Some(sea_orm::SqlErr::UniqueConstraintViolation(ref s)) if s.contains("uq_branches_tenant_id_name") || s.contains("name"))
            || e.to_string().contains("uq_branches_tenant_id_name");
        if is_name_conflict {
            AppError::conflict_code(
                "duplicate_branch_name",
                "A branch with this name already exists in this pharmacy",
            )
        } else {
            db_error_to_app_error(e)
        }
    })?;

    Ok(model_to_branch(updated))
}

pub async fn get_subscriptions(
    db: &Db,
    tenant: TenantId,
    pagination: Pagination,
) -> Result<ListPage<Subscription>, AppError> {
    let ctx = TenantContext::new(tenant, UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await?;
    let conn = tx.transaction()?;

    let limit = (pagination.limit as u64).clamp(1, 100);
    let mut query = subscriptions::Entity::find()
        .filter(subscriptions::Column::TenantId.eq(tenant.as_uuid()))
        .order_by_desc(subscriptions::Column::StartsAt);

    if let Some(cursor_str) = pagination.cursor {
        if let Ok(cursor_dt) = DateTime::parse_from_rfc3339(&cursor_str) {
            query = query.filter(subscriptions::Column::StartsAt.lt(cursor_dt.with_timezone(&Utc)));
        }
    }

    let rows = query
        .limit(limit + 1)
        .all(conn)
        .await
        .map_err(db_error_to_app_error)?;
    tx.commit().await?;

    let next_cursor = if rows.len() > limit as usize {
        rows.get(limit as usize - 1)
            .map(|r| r.starts_at.to_rfc3339())
    } else {
        None
    };
    let mut page_rows = rows;
    if page_rows.len() > limit as usize {
        page_rows.truncate(limit as usize);
    }

    let all_plans = plans::Entity::find()
        .all(db.conn())
        .await
        .map_err(db_error_to_app_error)?;
    let plan_map: std::collections::HashMap<Uuid, String> =
        all_plans.into_iter().map(|p| (p.id, p.code)).collect();

    let data = page_rows
        .into_iter()
        .map(|s| {
            let code = plan_map
                .get(&s.plan_id)
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            Subscription {
                id: s.id,
                tenant_id: TenantId::from_uuid(s.tenant_id),
                plan_code: code,
                billing_period: s.billing_period,
                price: s.price,
                starts_at: s.starts_at.with_timezone(&Utc),
                ends_at: s.ends_at.with_timezone(&Utc),
                status: s.status,
                is_complimentary: s.is_complimentary,
            }
        })
        .collect();

    Ok(ListPage::new(data, next_cursor))
}

pub async fn get_current_subscription(
    db: &Db,
    tenant: TenantId,
) -> Result<Option<Subscription>, AppError> {
    let ctx = TenantContext::new(tenant, UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await?;
    let conn = tx.transaction()?;
    let now = Utc::now();

    let sub = subscriptions::Entity::find()
        .filter(subscriptions::Column::TenantId.eq(tenant.as_uuid()))
        .filter(subscriptions::Column::Status.eq("active"))
        .filter(subscriptions::Column::StartsAt.lte(now))
        .filter(subscriptions::Column::EndsAt.gte(now))
        .order_by_desc(subscriptions::Column::EndsAt)
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?;

    tx.commit().await?;

    match sub {
        Some(s) => {
            let plan = plans::Entity::find_by_id(s.plan_id)
                .one(db.conn())
                .await
                .map_err(db_error_to_app_error)?
                .map(|p| p.code)
                .unwrap_or_else(|| "unknown".to_string());
            Ok(Some(Subscription {
                id: s.id,
                tenant_id: TenantId::from_uuid(s.tenant_id),
                plan_code: plan,
                billing_period: s.billing_period,
                price: s.price,
                starts_at: s.starts_at.with_timezone(&Utc),
                ends_at: s.ends_at.with_timezone(&Utc),
                status: s.status,
                is_complimentary: s.is_complimentary,
            }))
        }
        None => Ok(None),
    }
}

pub async fn upload_payment_slip(
    tx: &Tx,
    payment_id: Uuid,
    proof_file_key: String,
) -> Result<Payment, AppError> {
    let conn = tx.transaction()?;
    let tenant_id = tx.tenant_id()?;

    let pay = subscription_payments::Entity::find_by_id(payment_id)
        .filter(subscription_payments::Column::TenantId.eq(tenant_id.as_uuid()))
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Payment not found"))?;

    if pay.status != "submitted" {
        return Err(AppError::conflict_code(
            "already_reviewed",
            "Cannot upload slip for an already reviewed payment",
        ));
    }

    let mut active: subscription_payments::ActiveModel = pay.into();
    active.proof_file_key = Set(Some(proof_file_key));
    active.updated_at = Set(Utc::now().into());

    let updated = active.update(conn).await.map_err(db_error_to_app_error)?;
    Ok(model_to_payment(updated))
}

pub async fn request_export(tx: &Tx, user_id: UserId) -> Result<TenantExport, AppError> {
    let conn = tx.transaction()?;
    let tenant_id = tx.tenant_id()?;

    let active_export = tenant_exports::Entity::find()
        .filter(tenant_exports::Column::TenantId.eq(tenant_id.as_uuid()))
        .filter(tenant_exports::Column::Status.is_in(["pending", "processing"]))
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?;

    if active_export.is_some() {
        return Err(AppError::conflict_code(
            "export_already_running",
            "An export request is already in progress for this pharmacy",
        ));
    }

    let now = Utc::now();
    let export_id = Uuid::now_v7();
    let model = tenant_exports::ActiveModel {
        id: Set(export_id),
        tenant_id: Set(tenant_id.as_uuid()),
        requested_by_user_id: Set(user_id.as_uuid()),
        status: Set("pending".to_string()),
        file_key: Set(None),
        error_message: Set(None),
        ready_at: Set(None),
        expires_at: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    };

    let inserted = model.insert(conn).await.map_err(db_error_to_app_error)?;
    Ok(TenantExport {
        id: inserted.id,
        tenant_id: TenantId::from_uuid(inserted.tenant_id),
        status: inserted.status,
        download_url: None,
        error_message: None,
        created_at: inserted.created_at.with_timezone(&Utc),
        ready_at: None,
        expires_at: None,
    })
}

pub async fn get_export(
    db: &Db,
    tenant_id: TenantId,
    id: Uuid,
) -> Result<TenantExport, AppError> {
    let ctx = TenantContext::new(tenant_id, UserId::from_uuid(Uuid::nil()), vec![]);
    let tx = Tx::begin(db.conn(), &ctx).await?;
    let conn = tx.transaction()?;

    let export = tenant_exports::Entity::find_by_id(id)
        .filter(tenant_exports::Column::TenantId.eq(tenant_id.as_uuid()))
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Export not found"))?;

    tx.commit().await?;

    let download_url = if export.status == "ready" {
        export.file_key.as_ref().map(|k| format!("/api/v1/exports/download/{k}"))
    } else {
        None
    };

    Ok(TenantExport {
        id: export.id,
        tenant_id: TenantId::from_uuid(export.tenant_id),
        status: export.status,
        download_url,
        error_message: export.error_message,
        created_at: export.created_at.with_timezone(&Utc),
        ready_at: export.ready_at.map(|dt| dt.with_timezone(&Utc)),
        expires_at: export.expires_at.map(|dt| dt.with_timezone(&Utc)),
    })
}

pub async fn list_tenants(
    db: &Db,
    status_filter: Option<String>,
    pagination: Pagination,
) -> Result<ListPage<TenantSummary>, AppError> {
    let limit = (pagination.limit as u64).clamp(1, 100);
    let mut query = tenants::Entity::find().order_by_desc(tenants::Column::CreatedAt);

    if let Some(status) = status_filter {
        query = query.filter(tenants::Column::Status.eq(status));
    }

    if let Some(cursor_str) = pagination.cursor {
        if let Ok(cursor_dt) = DateTime::parse_from_rfc3339(&cursor_str) {
            query = query.filter(tenants::Column::CreatedAt.lt(cursor_dt.with_timezone(&Utc)));
        }
    }

    let rows = query
        .limit(limit + 1)
        .all(db.conn())
        .await
        .map_err(db_error_to_app_error)?;

    let next_cursor = if rows.len() > limit as usize {
        rows.get(limit as usize - 1)
            .map(|r| r.created_at.to_rfc3339())
    } else {
        None
    };
    let mut page_rows = rows;
    if page_rows.len() > limit as usize {
        page_rows.truncate(limit as usize);
    }

    let data = page_rows
        .into_iter()
        .map(|t| TenantSummary {
            id: TenantId::from_uuid(t.id),
            name: t.name,
            status: t.status,
            created_at: t.created_at.with_timezone(&Utc),
        })
        .collect();

    Ok(ListPage::new(data, next_cursor))
}

pub async fn get_tenant_detail(tx: &Tx, id: TenantId) -> Result<TenantDetail, AppError> {
    let conn = tx.transaction()?;
    let t = tenants::Entity::find_by_id(id.as_uuid())
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Tenant not found"))?;

    let mode = get_access_mode_with_conn(conn, id, Utc::now()).await?;
    let access = access_mode_to_summary(&mode);
    let tenant_dto = model_to_tenant(t, access);

    let settings = tenant_settings::Entity::find_by_id(id.as_uuid())
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .map(model_to_settings);

    let branch_rows = branches::Entity::find()
        .filter(branches::Column::TenantId.eq(id.as_uuid()))
        .order_by_asc(branches::Column::CreatedAt)
        .all(conn)
        .await
        .map_err(db_error_to_app_error)?;
    let branches = branch_rows.into_iter().map(model_to_branch).collect();

    let sub_rows = subscriptions::Entity::find()
        .filter(subscriptions::Column::TenantId.eq(id.as_uuid()))
        .order_by_desc(subscriptions::Column::StartsAt)
        .all(conn)
        .await
        .map_err(db_error_to_app_error)?;

    let all_plans = plans::Entity::find()
        .all(conn)
        .await
        .map_err(db_error_to_app_error)?;
    let plan_map: std::collections::HashMap<Uuid, String> =
        all_plans.into_iter().map(|p| (p.id, p.code)).collect();

    let subscriptions = sub_rows
        .into_iter()
        .map(|s| {
            let code = plan_map
                .get(&s.plan_id)
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            Subscription {
                id: s.id,
                tenant_id: TenantId::from_uuid(s.tenant_id),
                plan_code: code,
                billing_period: s.billing_period,
                price: s.price,
                starts_at: s.starts_at.with_timezone(&Utc),
                ends_at: s.ends_at.with_timezone(&Utc),
                status: s.status,
                is_complimentary: s.is_complimentary,
            }
        })
        .collect();

    Ok(TenantDetail {
        tenant: tenant_dto,
        settings,
        branches,
        subscriptions,
    })
}

pub async fn create_plan(tx: &Tx, req: NewPlanRequest) -> Result<Plan, AppError> {
    let conn = tx.transaction()?;
    let plan_id = Uuid::now_v7();
    let now = Utc::now();

    let active_plan = plans::ActiveModel {
        id: Set(plan_id),
        code: Set(req.code.clone()),
        name: Set(req.name.clone()),
        grace_days: Set(req.grace_days),
        is_active: Set(req.is_active),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    };

    let p = active_plan.insert(conn).await.map_err(|e| {
        let is_code_conflict = matches!(e.sql_err(), Some(sea_orm::SqlErr::UniqueConstraintViolation(ref s)) if s.contains("uq_plans_code") || s.contains("code"))
            || e.to_string().contains("uq_plans_code");
        if is_code_conflict {
            AppError::conflict_code("duplicate_plan_code", "A plan with this code already exists")
        } else {
            db_error_to_app_error(e)
        }
    })?;

    let mut prices = Vec::new();
    for pr in req.prices {
        let active_price = plan_prices::ActiveModel {
            plan_id: Set(plan_id),
            billing_period: Set(pr.billing_period.clone()),
            price: Set(pr.price),
            created_at: Set(now.into()),
        };
        active_price.insert(conn).await.map_err(db_error_to_app_error)?;
        prices.push(PlanPrice {
            billing_period: pr.billing_period,
            price: pr.price,
        });
    }

    // Every plan holds a row for every limit type (`plan_limit_types`) [AC-U09]
    let limit_types = plan_limit_types::Entity::find()
        .all(conn)
        .await
        .map_err(db_error_to_app_error)?;
    let mut limits = Vec::new();
    for lt in limit_types {
        let active_lim = plan_limits::ActiveModel {
            plan_id: Set(plan_id),
            limit_key: Set(lt.key.clone()),
            max_value: Set(None), // Unlimited by default [D-89]
        };
        active_lim.insert(conn).await.map_err(db_error_to_app_error)?;
        limits.push(PlanLimit {
            limit_key: lt.key,
            max_value: None,
        });
    }

    Ok(Plan {
        id: p.id,
        code: p.code,
        name: p.name,
        grace_days: p.grace_days,
        is_active: p.is_active,
        prices,
        limits,
    })
}

pub async fn update_plan(tx: &Tx, id: Uuid, req: UpdatePlanRequest) -> Result<Plan, AppError> {
    let conn = tx.transaction()?;
    let plan = plans::Entity::find_by_id(id)
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Plan not found"))?;

    let mut active: plans::ActiveModel = plan.into();
    if let Some(name) = req.name {
        active.name = Set(name);
    }
    if let Some(grace_days) = req.grace_days {
        active.grace_days = Set(grace_days);
    }
    if let Some(is_active) = req.is_active {
        active.is_active = Set(is_active);
    }
    active.updated_at = Set(Utc::now().into());

    let updated = active.update(conn).await.map_err(db_error_to_app_error)?;

    let prices = plan_prices::Entity::find()
        .filter(plan_prices::Column::PlanId.eq(id))
        .all(conn)
        .await
        .map_err(db_error_to_app_error)?
        .into_iter()
        .map(|pr| PlanPrice {
            billing_period: pr.billing_period,
            price: pr.price,
        })
        .collect();

    let limits = plan_limits::Entity::find()
        .filter(plan_limits::Column::PlanId.eq(id))
        .all(conn)
        .await
        .map_err(db_error_to_app_error)?
        .into_iter()
        .map(|li| PlanLimit {
            limit_key: li.limit_key,
            max_value: li.max_value,
        })
        .collect();

    Ok(Plan {
        id: updated.id,
        code: updated.code,
        name: updated.name,
        grace_days: updated.grace_days,
        is_active: updated.is_active,
        prices,
        limits,
    })
}

pub async fn set_plan_limit(
    tx: &Tx,
    plan_id: Uuid,
    limit_key: String,
    max_value: Option<i32>,
) -> Result<PlanLimit, AppError> {
    let conn = tx.transaction()?;

    // 1. Verify limit_key exists in plan_limit_types
    let lt = plan_limit_types::Entity::find_by_id(limit_key.clone())
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found(format!("Limit type '{limit_key}' not found")))?;

    // 2. Verify plan exists
    let _plan = plans::Entity::find_by_id(plan_id)
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?
        .ok_or_else(|| AppError::not_found("Plan not found"))?;

    // 3. Upsert into plan_limits
    let existing = plan_limits::Entity::find_by_id((plan_id, lt.key.clone()))
        .one(conn)
        .await
        .map_err(db_error_to_app_error)?;

    if let Some(lim) = existing {
        let mut active: plan_limits::ActiveModel = lim.into();
        active.max_value = Set(max_value);
        active.update(conn).await.map_err(db_error_to_app_error)?;
    } else {
        let active = plan_limits::ActiveModel {
            plan_id: Set(plan_id),
            limit_key: Set(lt.key.clone()),
            max_value: Set(max_value),
        };
        active.insert(conn).await.map_err(db_error_to_app_error)?;
    }

    Ok(PlanLimit {
        limit_key: lt.key,
        max_value,
    })
}
