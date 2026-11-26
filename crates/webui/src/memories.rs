//! Memory browsing, unified people, sources and explicit portrait refresh.
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{QueryBuilder, Row, Sqlite};

pub mod activity;

type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;
fn error(status: StatusCode, message: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (status, Json(json!({"error":message.to_string()})))
}
fn db_error(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    error(StatusCode::INTERNAL_SERVER_ERROR, e)
}

#[derive(Default, Deserialize)]
pub struct MemQuery {
    owner_type: Option<String>,
    owner_id: Option<String>,
    person_id: Option<String>,
    chat_id: Option<String>,
    chat_type: Option<String>,
    platform: Option<String>,
    search: Option<String>,
    limit: Option<i64>,
    page: Option<i64>,
    page_size: Option<i64>,
}
impl MemQuery {
    fn validate(&self) -> Result<(), (StatusCode, Json<Value>)> {
        if self
            .owner_type
            .as_deref()
            .is_some_and(|v| !["person", "chat", "self"].contains(&v))
            || self
                .chat_type
                .as_deref()
                .is_some_and(|v| !["group", "private"].contains(&v))
            || self
                .platform
                .as_deref()
                .is_some_and(|v| !["onebot", "qq_official"].contains(&v))
            || self
                .search
                .as_ref()
                .is_some_and(|s| s.chars().count() > 100)
            || self.page.is_some_and(|p| p < 1 || p > i64::MAX / 96)
            || self.page_size.is_some_and(|p| !(1..=96).contains(&p))
            || [&self.owner_id, &self.person_id, &self.chat_id]
                .into_iter()
                .flatten()
                .any(|s| s.len() > 192)
        {
            return Err(error(StatusCode::BAD_REQUEST, "查询参数无效"));
        }
        Ok(())
    }
}

// Old person summaries have no conversation column. Never infer a source from nearby messages.
fn cte(summary: bool) -> String {
    let records = if summary {
        "SELECT id,owner_type,owner_id,summary AS content,'consolidation' AS source,created_at,created_at AS updated_at,
            date,period,msg_id_start AS source_msg_id,msg_id_end AS source_end_msg_id,
            CASE WHEN owner_type='chat' THEN owner_id END AS chat_id,NULL AS stored_chat_type FROM summaries
         UNION ALL SELECT -id,owner_type,owner_id,content,'consolidation',created_at,created_at,date,'daily',source_msg_id,source_end_msg_id,source_chat_id,source_chat_type FROM memory_diary_entries"
    } else {
        "SELECT id,owner_type,owner_id,content,source,created_at,updated_at,NULL AS date,NULL AS period,
            source_msg_id,source_end_msg_id,CASE WHEN owner_type='chat' THEN owner_id ELSE source_chat_id END AS chat_id,NULL AS stored_chat_type FROM long_memories"
    };
    format!("WITH records AS ({records}), enriched AS (
        SELECT r.*,COALESCE(NULLIF(p.display_name,''),CASE WHEN r.owner_type='self' THEN '云团' END) AS owner_name,
        COALESCE(d.directory_id,r.owner_id) AS directory_id,
        (SELECT platform FROM identities WHERE person_id=p.person_id ORDER BY id DESC LIMIT 1) AS platform,
        (SELECT platform_uid FROM identities WHERE person_id=p.person_id ORDER BY id DESC LIMIT 1) AS platform_uid,
        COALESCE(r.stored_chat_type,(SELECT chat_type FROM messages WHERE msg_id=r.source_msg_id AND chat_id=r.chat_id)) AS chat_type,
        (SELECT card FROM member_profiles WHERE person_id=r.owner_id AND chat_id=r.chat_id) AS card
        FROM records r LEFT JOIN memory_directory d ON d.person_id=r.owner_id AND r.owner_type='person'
        LEFT JOIN persons p ON p.person_id=CASE WHEN r.owner_type='self' THEN 'self' ELSE COALESCE(d.directory_id,r.owner_id) END)")
}

fn person_members(builder: &mut QueryBuilder<'static, Sqlite>, person: &str) {
    builder.push("(SELECT ").push_bind(person.to_owned())
        .push(" UNION ALL SELECT alias FROM memory_aliases WHERE person_id=")
        .push_bind(person.to_owned()).push(")");
}

fn filtered(summary: bool, q: &MemQuery, count: bool) -> QueryBuilder<'static, Sqlite> {
    let mut b = QueryBuilder::new(format!(
        "{} SELECT {} FROM enriched e WHERE trim(COALESCE(content,''))!=''
        AND lower(trim(content)) NOT IN ('null','undefined')",
        cte(summary),
        if count { "COUNT(*)" } else { "*" }
    ));
    for (column, value) in [
        ("owner_type", &q.owner_type),
        ("owner_id", &q.owner_id),
        ("chat_id", &q.chat_id),
        ("chat_type", &q.chat_type),
    ] {
        if let Some(value) = value.as_ref().filter(|s| !s.trim().is_empty()) {
            b.push(format!(" AND {column}="))
                .push_bind(value.trim().to_owned());
        }
    }
    if let Some(id) = &q.person_id {
        b.push(" AND ((owner_type='person' AND owner_id IN ");
        person_members(&mut b, id);
        b.push(") OR (owner_type='self' AND ")
            .push_bind(id.clone())
            .push("='self'))");
    }
    if let Some(platform) = q.platform.as_deref() {
        if platform == yuantuan_core::platform::QQ_OFFICIAL_ID {
            b.push(" AND (COALESCE(chat_id,'') LIKE 'qq-official_%' OR owner_id LIKE 'qq-official_%')");
        } else if platform == yuantuan_core::platform::ONEBOT_ID {
            b.push(" AND COALESCE(chat_id,'') NOT LIKE 'qq-official_%' AND owner_id NOT LIKE 'qq-official_%'");
        }
    }
    if let Some(term) = q
        .search
        .as_ref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
    {
        b.push(" AND (instr(lower(content),").push_bind(term.clone()).push(")>0 OR instr(lower(COALESCE(owner_name,'')),")
            .push_bind(term.clone()).push(")>0 OR EXISTS(SELECT 1 FROM persons original WHERE original.person_id=e.owner_id AND instr(lower(COALESCE(original.display_name,'')),")
            .push_bind(term.clone()).push(")>0) OR instr(lower(owner_id),").push_bind(term.clone()).push(")>0 OR instr(COALESCE(platform_uid,''),")
            .push_bind(term.clone()).push(")>0 OR EXISTS(SELECT 1 FROM member_profiles mp WHERE mp.person_id=e.owner_id AND instr(lower(COALESCE(mp.card,'')),")
            .push_bind(term).push(")>0))");
    }
    b
}

fn identity(
    resolver: Option<&yuantuan_core::memory::AvatarResolver>,
    id: &str,
    name: Option<String>,
    platform: Option<String>,
    platform_uid: Option<String>,
    kind: &str,
) -> Value {
    let platform_uid = platform_uid.filter(|s| !s.is_empty());
    let qq = match (platform.as_deref(), platform_uid.as_deref()) {
        (Some("qq"), Some(uid)) if uid.bytes().all(|c| c.is_ascii_digit()) => Some(uid.to_owned()),
        _ => None,
    };
    let avatar_url = match (resolver, platform.as_deref(), platform_uid.as_deref()) {
        (Some(f), Some(p), Some(uid)) => f(p, uid),
        _ => None,
    };
    let name = name.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| {
        if kind == "chat" {
            format!("会话 {id}")
        } else {
            "未知成员".into()
        }
    });
    json!({"id":id,"name":name,"platform":platform,"platform_uid":platform_uid,"qq":qq,"avatar_url":avatar_url,"kind":kind})
}
fn conversation(id: Option<String>, kind: Option<String>) -> Option<Value> {
    id.map(|id| {
        let label = match kind.as_deref() {
            Some("group") => format!("群聊 {id}"),
            Some("private") => format!("私聊 {id}"),
            _ => format!("会话 {id}"),
        };
        json!({"id":id,"chat_type":kind,"label":label})
    })
}

async fn list(state: AppState, q: MemQuery, summary: bool) -> ApiResult {
    q.validate()?;
    let mut conn = state.open_db().await.map_err(db_error)?;
    let mut tx = sqlx::Connection::begin(&mut *conn)
        .await
        .map_err(db_error)?;
    let memory_count: i64 = filtered(false, &q, true)
        .build_query_scalar()
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
    let summary_count: i64 = filtered(true, &q, true)
        .build_query_scalar()
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
    let total = if summary { summary_count } else { memory_count };
    let paginated = q.page.is_some() || q.page_size.is_some();
    let size = if paginated {
        q.page_size.unwrap_or(24)
    } else {
        q.limit.unwrap_or(100).clamp(1, 500)
    };
    let page = q.page.unwrap_or(1).min(((total - 1).max(0) / size) + 1);
    let mut b = filtered(summary, &q, false);
    b.push(if summary {
        " ORDER BY date DESC,updated_at DESC,id DESC LIMIT "
    } else {
        " ORDER BY updated_at DESC,id DESC LIMIT "
    })
    .push_bind(size)
    .push(" OFFSET ")
    .push_bind((page - 1) * size);
    let rows = b.build().fetch_all(&mut *tx).await.map_err(db_error)?;
    let items=rows.iter().map(|r| -> sqlx::Result<Value> {
        let owner_id:String=r.try_get("owner_id")?;
        let directory_id:String=r.try_get("directory_id")?;
        let owner_type:String=r.try_get("owner_type")?;
        let content:String=r.try_get("content")?;
        let mut item=json!({"id":r.try_get::<i64,_>("id")?,"owner_type":owner_type,"owner_id":owner_id,
            "owner":identity(state.extras.avatar_resolver.as_ref(),&directory_id,r.try_get("owner_name")?,r.try_get("platform")?,r.try_get("platform_uid")?,&owner_type),
            "content":content,"source":r.try_get::<String,_>("source")?,"created_at":r.try_get::<i64,_>("created_at")?,
            "updated_at":r.try_get::<i64,_>("updated_at")?,"conversation":conversation(r.try_get("chat_id")?,r.try_get("chat_type")?),
            "source_msg_id":r.try_get::<Option<i64>,_>("source_msg_id")?,"source_end_msg_id":r.try_get::<Option<i64>,_>("source_end_msg_id")?,
            "group_card":r.try_get::<Option<String>,_>("card")?});
        if summary {item["summary"]=json!(content);item["date"]=json!(r.try_get::<String,_>("date")?);
            item["period"]=json!(r.try_get::<String,_>("period")?);
            item["msg_id_start"]=item["source_msg_id"].clone();item["msg_id_end"]=item["source_end_msg_id"].clone();}
        Ok(item)
    }).collect::<sqlx::Result<Vec<_>>>().map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    let mut data = json!({"total":total,"page":page,"page_size":size,"counts":{"memories":memory_count,"summaries":summary_count}});
    data[if summary { "summaries" } else { "memories" }] = json!(items);
    Ok(Json(data))
}
pub async fn memories(State(state): State<AppState>, Query(q): Query<MemQuery>) -> ApiResult {
    list(state, q, false).await
}
pub async fn summaries(State(state): State<AppState>, Query(q): Query<MemQuery>) -> ApiResult {
    list(state, q, true).await
}

fn people_filtered(q: &MemQuery, count: bool) -> QueryBuilder<'static, Sqlite> {
    let mut b = QueryBuilder::<Sqlite>::new(
        "WITH matched AS (SELECT d.directory_id,MAX(p.last_seen) AS last_seen FROM persons p
         JOIN memory_directory d ON d.person_id=p.person_id WHERE 1=1",
    );
    if let Some(platform) = q.platform.as_deref() {
        if platform == yuantuan_core::platform::QQ_OFFICIAL_ID {
            b.push(" AND p.person_id LIKE 'qq-official_%'");
        } else if platform == yuantuan_core::platform::ONEBOT_ID {
            b.push(" AND p.person_id NOT LIKE 'qq-official_%'");
        }
    }
    if let Some(chat) = &q.chat_id {
        b.push(
            " AND EXISTS(SELECT 1 FROM messages m WHERE m.sender_pid=p.person_id AND m.chat_id=",
        )
        .push_bind(chat.clone());
        if let Some(kind) = &q.chat_type {
            b.push(" AND m.chat_type=").push_bind(kind.clone());
        }
        b.push(")");
    }
    if let Some(term) = q
        .search
        .as_ref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
    {
        b.push(" AND (instr(lower(COALESCE(p.display_name,'')),").push_bind(term.clone()).push(")>0 OR instr(lower(p.person_id),").push_bind(term.clone())
            .push(")>0 OR EXISTS(SELECT 1 FROM identities i WHERE i.person_id=p.person_id AND instr(lower(i.platform_uid),").push_bind(term.clone()).push(")>0)")
            .push(" OR EXISTS(SELECT 1 FROM member_profiles mp WHERE mp.person_id=p.person_id AND instr(lower(COALESCE(mp.card,'')),").push_bind(term.clone()).push(")>0)")
            .push(" OR EXISTS(SELECT 1 FROM long_memories lm WHERE lm.owner_type='person' AND lm.owner_id=p.person_id AND instr(lower(COALESCE(lm.content,'')),").push_bind(term.clone()).push(")>0)")
            .push(" OR EXISTS(SELECT 1 FROM summaries s WHERE s.owner_type='person' AND s.owner_id=p.person_id AND instr(lower(COALESCE(s.summary,'')),").push_bind(term.clone()).push(")>0)")
            .push(" OR EXISTS(SELECT 1 FROM person_profile_facts pf WHERE pf.person_id=p.person_id AND instr(lower(pf.content),").push_bind(term.clone()).push(")>0)")
            .push(" OR EXISTS(SELECT 1 FROM memory_items mi WHERE mi.owner_type='person' AND mi.owner_id=p.person_id AND instr(lower(mi.content),").push_bind(term).push(")>0))");
    }
    b.push(" GROUP BY d.directory_id) ");
    if count {
        b.push("SELECT COUNT(*) FROM matched");
    } else {
        b.push("SELECT p.person_id,COALESCE(NULLIF(trim(p.display_name),''),(
            SELECT peer.display_name FROM memory_directory d JOIN persons peer ON peer.person_id=d.person_id
            WHERE d.directory_id=p.person_id AND trim(peer.display_name)<>''
            ORDER BY peer.last_seen DESC,peer.person_id LIMIT 1)) AS display_name,matched.last_seen,
            (SELECT platform FROM identities WHERE person_id=p.person_id ORDER BY id DESC LIMIT 1) AS platform,
            (SELECT platform_uid FROM identities WHERE person_id=p.person_id ORDER BY id DESC LIMIT 1) AS platform_uid,
            (SELECT json_group_array(person_id) FROM memory_directory
             WHERE directory_id=p.person_id AND person_id<>p.person_id) AS aliases
            FROM matched JOIN persons p ON p.person_id=matched.directory_id");
    }
    b
}

pub async fn people(State(state): State<AppState>, Query(q): Query<MemQuery>) -> ApiResult {
    q.validate()?;
    let mut conn = state.open_db().await.map_err(db_error)?;
    let mut tx = sqlx::Connection::begin(&mut *conn)
        .await
        .map_err(db_error)?;
    let total: i64 = people_filtered(&q, true)
        .build_query_scalar()
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
    let size = q.page_size.unwrap_or(24);
    let page = q.page.unwrap_or(1).min((total - 1).max(0) / size + 1);
    let mut b = people_filtered(&q, false);
    b.push(" ORDER BY matched.last_seen DESC,p.person_id LIMIT ")
        .push_bind(size)
        .push(" OFFSET ")
        .push_bind((page - 1) * size);
    let rows = b.build().fetch_all(&mut *tx).await.map_err(db_error)?;
    let ids = rows
        .iter()
        .map(|r| r.try_get::<String, _>("person_id"))
        .collect::<sqlx::Result<Vec<_>>>()
        .map_err(db_error)?;
    let mut portraits = yuantuan_core::memory::directory_profiles(&mut tx, &ids, i64::MAX)
        .await
        .map_err(db_error)?;
    let previews=sqlx::query("WITH latest AS (SELECT i.*,d.directory_id,
        ROW_NUMBER() OVER(PARTITION BY i.owner_id,item_key,source_chat_type,source_chat_id ORDER BY source_end_id DESC,id DESC) AS rn
        FROM memory_items i JOIN memory_directory d ON d.person_id=i.owner_id
        WHERE owner_type='person' AND d.directory_id IN (SELECT value FROM json_each(?1))),
        ranked AS (SELECT *,ROW_NUMBER() OVER(PARTITION BY directory_id ORDER BY CASE WHEN item_key='summary' THEN 0 ELSE 1 END,source_end_id DESC,id DESC) AS person_rank
        FROM latest WHERE rn=1 AND status IN ('current','uncertain') AND (valid_until IS NULL OR valid_until>unixepoch()))
        SELECT directory_id,substr(content,1,300) AS content,recorded_at FROM ranked WHERE person_rank=1")
        .bind(serde_json::to_string(&ids).map_err(db_error)?).fetch_all(&mut *tx).await.map_err(db_error)?;
    let mut preview_by_person = std::collections::BTreeMap::new();
    for row in previews {
        let content: String = row.try_get("content").map_err(db_error)?;
        if !yuantuan_core::consolidation::is_sensitive(&content) {
            preview_by_person.insert(
                row.try_get::<String, _>("directory_id").map_err(db_error)?,
                (
                    content,
                    row.try_get::<i64, _>("recorded_at").map_err(db_error)?,
                ),
            );
        }
    }
    let items = rows
        .iter()
        .take(size as usize)
        .map(|r| -> sqlx::Result<Value> {
            let id: String = r.try_get("person_id")?;
            let mut person = identity(
                state.extras.avatar_resolver.as_ref(),
                &id,
                r.try_get("display_name")?,
                r.try_get("platform")?,
                r.try_get("platform_uid")?,
                "person",
            );
            let aliases: String = r.try_get("aliases")?;
            person["aliases"] = serde_json::from_str(&aliases).map_err(|err| sqlx::Error::Decode(Box::new(err)))?;
            let facts = portraits.remove(&id).unwrap_or_default();
            let preview = preview_by_person.remove(&id);
            person["portrait_preview"] = json!(preview.as_ref().map(|(text, _)| text));
            person["profile_updated_at"] = json!(facts
                .iter()
                .map(|f| f.updated_at)
                .chain(preview.iter().map(|(_, at)| *at))
                .max());
            person["profile"] = json!(facts);
            Ok(person)
        })
        .collect::<sqlx::Result<Vec<_>>>()
        .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(
        json!({"people":items,"page":page,"total":total,"has_more":page*size<total}),
    ))
}

pub async fn conversations(State(state): State<AppState>, Query(q): Query<MemQuery>) -> ApiResult {
    q.validate()?;
    let mut conn = state.open_db().await.map_err(db_error)?;
    let mut b=QueryBuilder::<Sqlite>::new("SELECT chat_id,chat_type,MAX(ts) AS last_seen FROM messages WHERE chat_type IN ('group','private')");
    if let Some(person) = q.person_id {
        b.push(" AND sender_pid IN ");
        person_members(&mut b, &person);
    }
    b.push(" GROUP BY chat_id,chat_type ORDER BY last_seen DESC,chat_id LIMIT 200");
    let rows = b.build().fetch_all(&mut *conn).await.map_err(db_error)?;
    let items = rows
        .iter()
        .map(|r| -> sqlx::Result<Value> {
            Ok(conversation(Some(r.try_get("chat_id")?), Some(r.try_get("chat_type")?)).unwrap())
        })
        .collect::<sqlx::Result<Vec<_>>>()
        .map_err(db_error)?;
    Ok(Json(json!({"conversations":items})))
}

#[derive(Deserialize)]
pub struct SourceQuery {
    kind: Option<String>,
}
pub async fn source(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<SourceQuery>,
) -> ApiResult {
    if q.kind.as_deref() == Some("profile") {
        return activity::profile_source(&state, id).await;
    }
    let summary = match q.kind.as_deref().unwrap_or("memory") {
        "memory" => false,
        "summary" => true,
        _ => return Err(error(StatusCode::BAD_REQUEST, "来源类型无效")),
    };
    let mut conn = state.open_db().await.map_err(db_error)?;
    let row = sqlx::query(&format!(
        "{} SELECT * FROM enriched WHERE id=?1",
        cte(summary)
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(db_error)?
    .ok_or_else(|| error(StatusCode::NOT_FOUND, "记录不存在"))?;
    let chat: Option<String> = row.try_get("chat_id").map_err(db_error)?;
    let kind: Option<String> = row.try_get("chat_type").map_err(db_error)?;
    let start: Option<i64> = row.try_get("source_msg_id").map_err(db_error)?;
    let end: Option<i64> = row.try_get("source_end_msg_id").map_err(db_error)?;
    let (Some(chat), Some(kind), Some(start)) = (chat, kind, start) else {
        return Ok(Json(json!({"messages":[],"total":0,"conversation":null})));
    };
    if end.is_some_and(|e| e < start) {
        return Ok(Json(json!({"messages":[],"total":0,"conversation":null})));
    }
    let mut b=QueryBuilder::<Sqlite>::new("SELECT m.msg_id,m.sender_pid,m.nickname,m.text,m.ts,m.has_image,
        (SELECT display_name FROM persons WHERE person_id=m.sender_pid) AS name FROM messages m WHERE m.chat_id=");
    b.push_bind(chat.clone())
        .push(" AND m.chat_type=")
        .push_bind(kind.clone())
        .push(" AND m.msg_id BETWEEN ")
        .push_bind(start)
        .push(" AND ")
        .push_bind(end.unwrap_or(start));
    let owner_type: String = row.try_get("owner_type").map_err(db_error)?;
    if owner_type == "person" {
        b.push(" AND m.sender_pid=")
            .push_bind(row.try_get::<String, _>("owner_id").map_err(db_error)?);
    }
    b.push(" ORDER BY m.msg_id LIMIT 61");
    let rows = b.build().fetch_all(&mut *conn).await.map_err(db_error)?;
    let messages=rows.iter().take(60).map(|r|->sqlx::Result<Value>{
        let text:Option<String>=r.try_get("text")?;
        let text=text.unwrap_or_default();
        let text=if yuantuan_core::consolidation::is_sensitive(&text) {"敏感内容已隐藏".into()} else {text.chars().take(4000).collect::<String>()};
        Ok(json!({"msg_id":r.try_get::<i64,_>("msg_id")?,"person_id":r.try_get::<String,_>("sender_pid")?,
            "name":r.try_get::<Option<String>,_>("nickname")?.filter(|s|!s.is_empty()).or(r.try_get::<Option<String>,_>("name")?),"text":text,"ts":r.try_get::<i64,_>("ts")?,"has_image":r.try_get::<bool,_>("has_image")?}))
    }).collect::<sqlx::Result<Vec<_>>>().map_err(db_error)?;
    Ok(Json(
        json!({"messages":messages,"has_more":rows.len()>60,"conversation":conversation(Some(chat),Some(kind)),"is_window":end.is_some()}),
    ))
}

/// Explicit administrator action; browsing remains read-only and never starts a model call.
pub async fn refresh_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut conn = state
        .open_db()
        .await
        .map_err(|_| db_error("数据库读取失败"))?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM persons WHERE person_id=?1 AND person_id<>'self')",
    )
    .bind(&id)
    .fetch_one(&mut *conn)
    .await
    .map_err(|_| db_error("人物读取失败"))?;
    if !exists {
        return Err((StatusCode::NOT_FOUND, Json(json!({"error":"人物不存在"}))));
    }
    let subject: String = sqlx::query_scalar(
        "SELECT COALESCE((SELECT m.sender_pid FROM messages m
          LEFT JOIN memory_progress p ON p.owner_id=m.sender_pid AND p.stage='portrait' AND p.chat_type=''
          LEFT JOIN portrait_coverage c ON c.person_id=m.sender_pid AND c.msg_id=m.msg_id
          WHERE m.sender_pid IN (SELECT ?1 UNION ALL SELECT alias FROM memory_aliases WHERE person_id=?1)
            AND m.chat_type IN ('group','private') AND m.sender_bot=0 AND length(trim(COALESCE(m.text,'')))>=4
            AND m.msg_id>COALESCE(p.cursor,0) AND c.msg_id IS NULL
          ORDER BY m.msg_id DESC LIMIT 1),?1)",
    )
    .bind(&id)
    .fetch_one(&mut *conn)
    .await
    .map_err(|_| db_error("人物读取失败"))?;
    drop(conn);
    let gateway = state
        .extras
        .llm_slot
        .read()
        .unwrap()
        .clone()
        .filter(|g| g.role(yuantuan_core::llm::Role::Decision).is_some())
        .ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error":"未配置 Decision 模型"})),
            )
        })?;
    let result = yuantuan_core::memory::refresh::person(&state.db_path, &gateway, &subject, true)
        .await
        .map_err(|e| {
            tracing::warn!(error=%e,"人物画像整理失败");
            db_error("画像整理失败，请稍后重试")
        })?;
    Ok(Json(json!(result)))
}
