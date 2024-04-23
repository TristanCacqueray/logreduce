// Copyright (C) 2023 Red Hat
// SPDX-License-Identifier: Apache-2.0

//! This module contains the database logic.

// provides `try_next`
use futures::TryStreamExt;

use logjuicer_model::MODEL_VERSION;
use sqlx::types::chrono::Utc;

use logjuicer_report::{
    model_row::{ContentID, ModelRow},
    report_row::{FileSize, ReportID, ReportRow, ReportStatus},
};

#[derive(Clone)]
pub struct Db(sqlx::SqlitePool);

const MODEL_VER: i64 = MODEL_VERSION as i64;

impl Db {
    pub async fn new(storage_dir: &str) -> sqlx::Result<Db> {
        let db_url = format!("sqlite://{storage_dir}/logjuicer.sqlite?mode=rwc");
        let pool = sqlx::SqlitePool::connect(&db_url).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        let db = Db(pool);
        db.clean_pending().await?;
        db.clean_old_models(storage_dir).await?;
        Ok(db)
    }

    async fn clean_pending(&self) -> sqlx::Result<()> {
        let status = ReportStatus::Pending.as_str();
        sqlx::query!("delete from reports where status = ?", status)
            .execute(&self.0)
            .await
            .map(|_| ())
    }

    #[cfg(test)]
    pub async fn deprecate_models(&self) -> sqlx::Result<()> {
        sqlx::query!("update models set version = 0")
            .execute(&self.0)
            .await
            .map(|_| ())
    }

    #[cfg(test)]
    pub async fn test_clean_old_models(&self, storage_dir: &str) -> sqlx::Result<()> {
        self.clean_old_models(storage_dir).await
    }

    async fn clean_old_models(&self, storage_dir: &str) -> sqlx::Result<()> {
        let mut rows = sqlx::query!(
            "select content_id from models where version != ?",
            MODEL_VER
        )
        .map(|row| row.content_id.into())
        .fetch(&self.0);
        let mut clean_count = 0;
        while let Some(row) = rows.try_next().await? {
            crate::models::delete_model(storage_dir, &row);
            clean_count += 1;
        }
        if clean_count > 0 {
            tracing::info!(count = clean_count, "Cleaned old models");
        }
        sqlx::query!("delete from models where version != ?", MODEL_VER)
            .execute(&self.0)
            .await
            .map(|_| ())
    }

    pub async fn get_reports(&self) -> sqlx::Result<Vec<ReportRow>> {
        sqlx::query_as!(
        ReportRow,
        "select id, created_at, updated_at, target, baseline, anomaly_count, status, bytes_size from reports order by id desc"
    )
        .fetch_all(&self.0)
        .await
    }

    pub async fn get_report_status(
        &self,
        report_id: ReportID,
    ) -> sqlx::Result<Option<ReportStatus>> {
        sqlx::query!("select status from reports where id = ?", report_id.0)
            .map(|row| row.status.into())
            .fetch_optional(&self.0)
            .await
    }

    pub async fn lookup_report(
        &self,
        target: &str,
        baseline: &str,
    ) -> sqlx::Result<Option<(ReportID, ReportStatus)>> {
        sqlx::query!(
            "select id, status from reports where target = ? and baseline = ?",
            target,
            baseline
        )
        .map(|row| (row.id.into(), row.status.into()))
        .fetch_optional(&self.0)
        .await
    }

    pub async fn update_report(
        &self,
        report_id: ReportID,
        anomaly_count: usize,
        status: &ReportStatus,
        size: FileSize,
    ) -> sqlx::Result<()> {
        let now = Utc::now();
        let count = anomaly_count as i64;
        let size = size.0 as i64;
        let status = status.as_str();
        sqlx::query!(
            "update reports set updated_at = ?, anomaly_count = ?, status = ?, bytes_size = ? where id = ?",
            now,
            count,
            status,
            size,
            report_id.0,
        )
        .execute(&self.0)
        .await
        .map(|_| ())
    }

    pub async fn initialize_report(&self, target: &str, baseline: &str) -> sqlx::Result<ReportID> {
        let now_utc = Utc::now();
        let status = ReportStatus::Pending.as_str();
        let id = sqlx::query!(
            "insert into reports (created_at, updated_at, target, baseline, anomaly_count, status)
                      values (?, ?, ?, ?, ?, ?)",
            now_utc,
            now_utc,
            target,
            baseline,
            0,
            status
        )
        .execute(&self.0)
        .await?
        .last_insert_rowid();
        Ok(id.into())
    }

    pub async fn get_models(&self) -> sqlx::Result<Vec<ModelRow>> {
        sqlx::query_as!(
            ModelRow,
            "select content_id, version, created_at, bytes_size from models"
        )
        .fetch_all(&self.0)
        .await
    }

    pub async fn lookup_model(&self, content_id: &ContentID) -> sqlx::Result<Option<()>> {
        sqlx::query!(
            "select version from models where content_id = ?",
            content_id.0
        )
        .map(|_row| ())
        .fetch_optional(&self.0)
        .await
    }

    pub async fn add_model(&self, content_id: &ContentID, size: FileSize) -> sqlx::Result<()> {
        let now_utc = Utc::now();
        let size = size.0 as i64;
        sqlx::query!(
            "insert into models (content_id, version, created_at, bytes_size)
                      values (?, ?, ?, ?)",
            content_id.0,
            MODEL_VER,
            now_utc,
            size,
        )
        .execute(&self.0)
        .await
        .map(|_| ())
    }
}
