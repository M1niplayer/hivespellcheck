use chrono::Local;

use super::{GroupRelevance, RoleInGroup};
use crate::{
    errors::{AppError, AppResult},
    guards::{perms::PermsEvaluator, user::User},
    models::Group,
    perms::{GroupsScope, HivePermission, TagContent},
    services::{groups::AuthorityInGroup, pg_args},
    HIVE_SYSTEM_ID,
};

pub async fn get_one<'x, X>(id: &str, domain: &str, db: X) -> AppResult<Option<Group>>
where
    X: sqlx::Executor<'x, Database = sqlx::Postgres>,
{
    let group = sqlx::query_as("SELECT * FROM groups WHERE id = $1 AND domain = $2")
        .bind(id)
        .bind(domain)
        .fetch_optional(db)
        .await?;

    Ok(group)
}

pub async fn require_one<'x, X>(id: &str, domain: &str, db: X) -> AppResult<Group>
where
    X: sqlx::Executor<'x, Database = sqlx::Postgres>,
{
    get_one(id, domain, db)
        .await?
        .ok_or_else(|| AppError::NoSuchGroup(id.to_owned(), domain.to_owned()))
}

pub async fn get_relevance<'x, X>(
    id: &str,
    domain: &str,
    db: X,
    perms: &PermsEvaluator,
    user: &User,
) -> AppResult<Option<GroupRelevance>>
where
    X: sqlx::Executor<'x, Database = sqlx::Postgres> + Copy,
{
    let mut authority = AuthorityInGroup::None;

    try_authority_from_permissions(
        id,
        domain,
        HivePermission::ManageGroups(GroupsScope::Any),
        AuthorityInGroup::FullyAuthorized,
        &mut authority,
        db,
        perms,
    )
    .await?;

    try_authority_from_permissions(
        id,
        domain,
        HivePermission::ManageMembers(GroupsScope::Any),
        AuthorityInGroup::ManageMembers,
        &mut authority,
        db,
        perms,
    )
    .await?;

    Ok(GroupRelevance::new(
        get_role_in_group(&user.username, id, domain, db).await?,
        authority,
    ))
}

async fn try_authority_from_permissions<'x, X>(
    id: &str,
    domain: &str,
    probe: HivePermission,
    potential_value: AuthorityInGroup,
    authority: &mut AuthorityInGroup,
    db: X,
    perms: &PermsEvaluator,
) -> AppResult<()>
where
    X: sqlx::Executor<'x, Database = sqlx::Postgres>,
{
    if *authority >= potential_value {
        // no point doing complicated work if we wouldn't ever be able to
        // elevate authority from its current value
        return Ok(());
    }

    let mut tags = vec![];

    for perm in perms.fetch_all_related(probe).await? {
        if let HivePermission::ManageGroups(scope) | HivePermission::ManageMembers(scope) = perm {
            match scope {
                GroupsScope::Wildcard => {
                    *authority = potential_value;
                    break;
                }
                GroupsScope::Domain(d) if d == domain => {
                    *authority = potential_value;
                    break;
                }
                GroupsScope::Domain(_) => {}
                GroupsScope::Tag { id, content } => tags.push((id, content)),
                GroupsScope::Any => unreachable!(),
            }
        }
    }

    if *authority < potential_value && has_any_tag(id, domain, &tags, db).await? {
        *authority = potential_value;
    }

    Ok(())
}

async fn has_any_tag<'x, X>(
    id: &str,
    domain: &str,
    tags: &[(String, Option<TagContent>)],
    db: X,
) -> AppResult<bool>
where
    X: sqlx::Executor<'x, Database = sqlx::Postgres>,
{
    if tags.is_empty() {
        return Ok(false);
    }

    let mut query = sqlx::QueryBuilder::with_arguments(
        "SELECT COUNT(*)
        FROM tag_assignments
        WHERE (group_id = $1 AND group_domain = $2)
            AND system_id = $3
            AND (",
        pg_args!(id, domain, HIVE_SYSTEM_ID),
    );

    for (i, (id, content)) in tags.iter().enumerate() {
        if i > 0 {
            query.push(" OR ");
        }
        match content {
            None | Some(TagContent::Wildcard) => {
                query.push("tag_id = ");
                query.push_bind(id);
            }
            Some(TagContent::Custom(content)) => {
                query.push("(tag_id = ");
                query.push_bind(id);
                query.push(" AND content = ");
                query.push_bind(content);
                query.push(")");
            }
        }
    }

    query.push(")");

    let count: i64 = query.build_query_scalar().fetch_one(db).await?;

    Ok(count > 0)
}

async fn get_role_in_group<'x, X>(
    username: &str,
    id: &str,
    domain: &str,
    db: X,
) -> AppResult<Option<RoleInGroup>>
where
    X: sqlx::Executor<'x, Database = sqlx::Postgres>,
{
    let today = Local::now().date_naive();

    let is_manager = sqlx::query_scalar(
        "SELECT manager
        FROM all_members_of($1, $2, $3)
        WHERE username = $4
        ORDER BY manager DESC
        LIMIT 1", // multiple paths may be possible; DESC makes true come first
    )
    .bind(id)
    .bind(domain)
    .bind(today)
    .bind(username)
    .fetch_optional(db)
    .await?;

    let role = match is_manager {
        Some(true) => Some(RoleInGroup::Manager),
        Some(false) => Some(RoleInGroup::Member),
        None => None,
    };

    Ok(role)
}
