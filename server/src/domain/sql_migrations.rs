use crate::domain::{
    sql_tables::{DbConnection, SchemaVersion},
    types::{GroupId, UserId, Uuid},
};
use sea_orm::{ConnectionTrait, FromQueryResult, Statement};
use sea_query::{ColumnDef, Expr, ForeignKey, ForeignKeyAction, Iden, Query, Table, Value};
use serde::{Deserialize, Serialize};
use tracing::{instrument, warn};

#[derive(Iden, PartialEq, Eq, Debug, Serialize, Deserialize, Clone)]
pub enum Users {
    Table,
    UserId,
    Email,
    DisplayName,
    FirstName,
    LastName,
    Avatar,
    CreationDate,
    PasswordHash,
    TotpSecret,
    MfaType,
    Uuid,
}

#[derive(Iden, PartialEq, Eq, Debug, Serialize, Deserialize, Clone)]
pub enum Groups {
    Table,
    GroupId,
    DisplayName,
    CreationDate,
    Uuid,
}

#[derive(Iden)]
pub enum Memberships {
    Table,
    UserId,
    GroupId,
}

// Metadata about the SQL DB.
#[derive(Iden)]
pub enum Metadata {
    Table,
    // Which version of the schema we're at.
    Version,
}

#[derive(FromQueryResult, PartialEq, Eq, Debug)]
pub struct JustSchemaVersion {
    pub version: SchemaVersion,
}

#[instrument(skip_all, level = "debug", ret)]
pub async fn get_schema_version(pool: &DbConnection) -> Option<SchemaVersion> {
    JustSchemaVersion::find_by_statement(
        pool.get_database_backend().build(
            Query::select()
                .from(Metadata::Table)
                .column(Metadata::Version),
        ),
    )
    .one(pool)
    .await
    .ok()
    .flatten()
    .map(|j| j.version)
}

pub async fn upgrade_to_v1(pool: &DbConnection) -> std::result::Result<(), sea_orm::DbErr> {
    let builder = pool.get_database_backend();
    // SQLite needs this pragma to be turned on. Other DB might not understand this, so ignore the
    // error.
    let _ = pool
        .execute(Statement::from_string(
            builder,
            "PRAGMA foreign_keys = ON".to_owned(),
        ))
        .await;

    pool.execute(
        builder.build(
            Table::create()
                .table(Users::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(Users::UserId)
                        .string_len(255)
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(Users::Email).string_len(255).not_null())
                .col(
                    ColumnDef::new(Users::DisplayName)
                        .string_len(255)
                        .not_null(),
                )
                .col(ColumnDef::new(Users::FirstName).string_len(255))
                .col(ColumnDef::new(Users::LastName).string_len(255))
                .col(ColumnDef::new(Users::Avatar).binary())
                .col(ColumnDef::new(Users::CreationDate).date_time().not_null())
                .col(ColumnDef::new(Users::PasswordHash).binary())
                .col(ColumnDef::new(Users::TotpSecret).string_len(64))
                .col(ColumnDef::new(Users::MfaType).string_len(64))
                .col(ColumnDef::new(Users::Uuid).string_len(36).not_null()),
        ),
    )
    .await?;

    pool.execute(
        builder.build(
            Table::create()
                .table(Groups::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(Groups::GroupId)
                        .integer()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(Groups::DisplayName)
                        .string_len(255)
                        .unique_key()
                        .not_null(),
                )
                .col(ColumnDef::new(Users::CreationDate).date_time().not_null())
                .col(ColumnDef::new(Users::Uuid).string_len(36).not_null()),
        ),
    )
    .await?;

    // If the creation_date column doesn't exist, add it.
    if pool
        .execute(
            builder.build(
                Table::alter().table(Groups::Table).add_column(
                    ColumnDef::new(Groups::CreationDate)
                        .date_time()
                        .not_null()
                        .default(chrono::Utc::now().naive_utc()),
                ),
            ),
        )
        .await
        .is_ok()
    {
        warn!("`creation_date` column not found in `groups`, creating it");
    }

    // If the uuid column doesn't exist, add it.
    if pool
        .execute(
            builder.build(
                Table::alter().table(Groups::Table).add_column(
                    ColumnDef::new(Groups::Uuid)
                        .string_len(36)
                        .not_null()
                        .default(""),
                ),
            ),
        )
        .await
        .is_ok()
    {
        warn!("`uuid` column not found in `groups`, creating it");
        #[derive(FromQueryResult)]
        struct ShortGroupDetails {
            group_id: GroupId,
            display_name: String,
            creation_date: chrono::DateTime<chrono::Utc>,
        }
        for result in ShortGroupDetails::find_by_statement(
            builder.build(
                Query::select()
                    .from(Groups::Table)
                    .column(Groups::GroupId)
                    .column(Groups::DisplayName)
                    .column(Groups::CreationDate),
            ),
        )
        .all(pool)
        .await?
        {
            pool.execute(
                builder.build(
                    Query::update()
                        .table(Groups::Table)
                        .value(
                            Groups::Uuid,
                            Value::from(Uuid::from_name_and_date(
                                &result.display_name,
                                &result.creation_date,
                            )),
                        )
                        .and_where(Expr::col(Groups::GroupId).eq(result.group_id)),
                ),
            )
            .await?;
        }
    }

    if pool
        .execute(
            builder.build(
                Table::alter().table(Users::Table).add_column(
                    ColumnDef::new(Users::Uuid)
                        .string_len(36)
                        .not_null()
                        .default(""),
                ),
            ),
        )
        .await
        .is_ok()
    {
        warn!("`uuid` column not found in `users`, creating it");
        #[derive(FromQueryResult)]
        struct ShortUserDetails {
            user_id: UserId,
            creation_date: chrono::DateTime<chrono::Utc>,
        }
        for result in ShortUserDetails::find_by_statement(
            builder.build(
                Query::select()
                    .from(Users::Table)
                    .column(Users::UserId)
                    .column(Users::CreationDate),
            ),
        )
        .all(pool)
        .await?
        {
            pool.execute(
                builder.build(
                    Query::update()
                        .table(Users::Table)
                        .value(
                            Users::Uuid,
                            Value::from(Uuid::from_name_and_date(
                                result.user_id.as_str(),
                                &result.creation_date,
                            )),
                        )
                        .and_where(Expr::col(Users::UserId).eq(result.user_id)),
                ),
            )
            .await?;
        }
    }

    pool.execute(
        builder.build(
            Table::create()
                .table(Memberships::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(Memberships::UserId)
                        .string_len(255)
                        .not_null(),
                )
                .col(ColumnDef::new(Memberships::GroupId).integer().not_null())
                .foreign_key(
                    ForeignKey::create()
                        .name("MembershipUserForeignKey")
                        .from(Memberships::Table, Memberships::UserId)
                        .to(Users::Table, Users::UserId)
                        .on_delete(ForeignKeyAction::Cascade)
                        .on_update(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("MembershipGroupForeignKey")
                        .from(Memberships::Table, Memberships::GroupId)
                        .to(Groups::Table, Groups::GroupId)
                        .on_delete(ForeignKeyAction::Cascade)
                        .on_update(ForeignKeyAction::Cascade),
                ),
        ),
    )
    .await?;

    if pool
        .query_one(
            builder.build(
                Query::select()
                    .from(Groups::Table)
                    .column(Groups::DisplayName)
                    .cond_where(Expr::col(Groups::DisplayName).eq("lldap_readonly")),
            ),
        )
        .await
        .is_ok()
    {
        pool.execute(
            builder.build(
                Query::update()
                    .table(Groups::Table)
                    .values(vec![(Groups::DisplayName, "lldap_password_manager".into())])
                    .cond_where(Expr::col(Groups::DisplayName).eq("lldap_readonly")),
            ),
        )
        .await?;
    }

    pool.execute(
        builder.build(
            Table::create()
                .table(Metadata::Table)
                .if_not_exists()
                .col(ColumnDef::new(Metadata::Version).tiny_integer()),
        ),
    )
    .await?;

    pool.execute(
        builder.build(
            Query::insert()
                .into_table(Metadata::Table)
                .columns(vec![Metadata::Version])
                .values_panic(vec![SchemaVersion(1).into()]),
        ),
    )
    .await?;

    assert_eq!(get_schema_version(pool).await.unwrap().0, 1);

    Ok(())
}

pub async fn migrate_from_version(
    _pool: &DbConnection,
    version: SchemaVersion,
) -> anyhow::Result<()> {
    if version.0 > 1 {
        anyhow::bail!("DB version downgrading is not supported");
    }
    Ok(())
}
