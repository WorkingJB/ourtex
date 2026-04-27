//! Desktop proposals review.
//!
//! Two backends:
//!   * **Local workspace** — proposals are JSON files dropped by stdio
//!     `orchext-mcp` under `<root>/.orchext/proposals/<id>.json`. Listing
//!     reads + parses them; approve/reject mutates the JSON in place
//!     (and applies the patch to the PlainFileDriver on approve).
//!   * **Remote workspace** — calls through to `orchext-sync`'s
//!     `RemoteClient::list_proposals` etc., which hit
//!     `/v1/t/:tid/proposals*`.
//!
//! The DTO surface (`Proposal`) is unified across both backends so the
//! React side renders the same way regardless of where the proposal
//! lives — that's the "experience stays as close to web-app as possible"
//! goal made concrete.

use crate::state::{AppState, Services};
use chrono::{DateTime, Utc};
use orchext_vault::{Document, DocumentId, Frontmatter};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub doc_id: String,
    pub base_version: String,
    pub patch: JsonValue,
    #[serde(default)]
    pub reason: Option<String>,
    pub status: String,
    #[serde(default)]
    pub actor_token_id: Option<String>,
    pub actor_token_label: String,
    #[serde(default)]
    pub actor_account_id: Option<String>,
    #[serde(default)]
    pub decided_by: Option<String>,
    #[serde(default)]
    pub decided_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub decision_note: Option<String>,
    #[serde(default)]
    pub applied_version: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[tauri::command]
pub async fn proposal_list(
    state: State<'_, AppState>,
    status: Option<String>,
) -> Result<Vec<Proposal>, String> {
    let svcs = state.active_services().await?;
    let filter = status.as_deref().unwrap_or("pending");
    if svcs.is_remote() {
        let client = svcs
            .remote_client
            .as_ref()
            .ok_or_else(|| "remote workspace missing client".to_string())?;
        let resp = client
            .list_proposals(filter)
            .await
            .map_err(|e| format!("list proposals: {e}"))?;
        return Ok(resp
            .proposals
            .into_iter()
            .map(remote_to_dto)
            .collect());
    }
    list_local(&svcs, filter).await
}

#[tauri::command]
pub async fn proposal_approve(
    state: State<'_, AppState>,
    id: String,
    note: Option<String>,
) -> Result<Proposal, String> {
    let svcs = state.active_services().await?;
    if svcs.is_remote() {
        let client = svcs
            .remote_client
            .as_ref()
            .ok_or_else(|| "remote workspace missing client".to_string())?;
        let resp = client
            .approve_proposal(&id, note.as_deref())
            .await
            .map_err(|e| format!("approve proposal: {e}"))?;
        return Ok(remote_to_dto(resp.proposal));
    }
    approve_local(&svcs, &id, note.as_deref()).await
}

#[tauri::command]
pub async fn proposal_reject(
    state: State<'_, AppState>,
    id: String,
    note: Option<String>,
) -> Result<Proposal, String> {
    let svcs = state.active_services().await?;
    if svcs.is_remote() {
        let client = svcs
            .remote_client
            .as_ref()
            .ok_or_else(|| "remote workspace missing client".to_string())?;
        let proposal = client
            .reject_proposal(&id, note.as_deref())
            .await
            .map_err(|e| format!("reject proposal: {e}"))?;
        return Ok(remote_to_dto(proposal));
    }
    reject_local(&svcs, &id, note.as_deref()).await
}

// ---------- local backend ----------

fn proposals_dir(root: &Path) -> PathBuf {
    root.join(".orchext").join("proposals")
}

async fn list_local(svcs: &Services, filter: &str) -> Result<Vec<Proposal>, String> {
    let dir = proposals_dir(&svcs.root);
    if !tokio::fs::try_exists(&dir).await.unwrap_or(false) {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    let mut rd = tokio::fs::read_dir(&dir)
        .await
        .map_err(|e| format!("read proposals dir: {e}"))?;
    while let Some(entry) = rd
        .next_entry()
        .await
        .map_err(|e| format!("scan proposals: {e}"))?
    {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        match read_local_proposal(&path).await {
            Ok(p) => {
                if matches_filter(&p.status, filter) {
                    out.push(p);
                }
            }
            Err(e) => tracing::warn!(path = %path.display(), err = %e, "skipping proposal"),
        }
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

async fn read_local_proposal(path: &Path) -> Result<Proposal, String> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    // Stdio writes use `proposal_id`; normalize to `id` and fill the
    // status field if absent (older drops were always pending).
    let raw: JsonValue =
        serde_json::from_slice(&bytes).map_err(|e| format!("parse {}: {e}", path.display()))?;

    let id = raw
        .get("id")
        .and_then(JsonValue::as_str)
        .or_else(|| raw.get("proposal_id").and_then(JsonValue::as_str))
        .ok_or_else(|| format!("{}: missing id", path.display()))?
        .to_string();
    let doc_id = raw
        .get("doc_id")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| format!("{}: missing doc_id", path.display()))?
        .to_string();
    let base_version = raw
        .get("base_version")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| format!("{}: missing base_version", path.display()))?
        .to_string();
    let patch = raw.get("patch").cloned().unwrap_or(JsonValue::Null);
    let reason = raw
        .get("reason")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let status = raw
        .get("status")
        .and_then(JsonValue::as_str)
        .unwrap_or("pending")
        .to_string();
    let actor_token_id = raw
        .get("actor_token_id")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let actor_token_label = raw
        .get("actor_token_label")
        .and_then(JsonValue::as_str)
        .unwrap_or("agent")
        .to_string();
    let decision_note = raw
        .get("decision_note")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let applied_version = raw
        .get("applied_version")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let created_at = raw
        .get("created_at")
        .and_then(JsonValue::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);
    let decided_at = raw
        .get("decided_at")
        .and_then(JsonValue::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));

    Ok(Proposal {
        id,
        doc_id,
        base_version,
        patch,
        reason,
        status,
        actor_token_id,
        actor_token_label,
        actor_account_id: None,
        decided_by: None,
        decided_at,
        decision_note,
        applied_version,
        created_at,
    })
}

fn matches_filter(status: &str, filter: &str) -> bool {
    match filter {
        "all" => true,
        "pending" => status == "pending",
        "approved" => status == "approved",
        "rejected" => status == "rejected",
        _ => false,
    }
}

async fn approve_local(svcs: &Services, id: &str, note: Option<&str>) -> Result<Proposal, String> {
    let path = proposals_dir(&svcs.root).join(format!("{id}.json"));
    let mut prop = read_local_proposal(&path).await?;
    if prop.status != "pending" {
        return Err("proposal already decided".into());
    }

    let doc_id = DocumentId::new(prop.doc_id.clone())
        .map_err(|e| format!("invalid doc_id {}: {e}", prop.doc_id))?;
    let current = svcs
        .vault
        .read(&doc_id)
        .await
        .map_err(|e| format!("read {doc_id}: {e}"))?;
    let current_version = current
        .version()
        .map_err(|e| format!("compute version: {e}"))?;
    if current_version != prop.base_version {
        return Err("version_conflict".into());
    }

    let new_doc = apply_patch_local(&current, &prop.patch)
        .map_err(|e| format!("apply patch: {e}"))?;
    let new_version = new_doc
        .version()
        .map_err(|e| format!("compute new version: {e}"))?;

    svcs.vault
        .write(&doc_id, &new_doc)
        .await
        .map_err(|e| format!("write {doc_id}: {e}"))?;
    svcs.index
        .upsert(&new_doc.frontmatter.type_, &new_doc)
        .await
        .map_err(|e| format!("index upsert: {e}"))?;

    let now = Utc::now();
    prop.status = "approved".into();
    prop.decided_at = Some(now);
    prop.decision_note = note.map(str::to_string);
    prop.applied_version = Some(new_version);
    write_local_proposal(&path, &prop).await?;
    Ok(prop)
}

async fn reject_local(svcs: &Services, id: &str, note: Option<&str>) -> Result<Proposal, String> {
    let path = proposals_dir(&svcs.root).join(format!("{id}.json"));
    let mut prop = read_local_proposal(&path).await?;
    if prop.status != "pending" {
        return Err("proposal already decided".into());
    }
    prop.status = "rejected".into();
    prop.decided_at = Some(Utc::now());
    prop.decision_note = note.map(str::to_string);
    write_local_proposal(&path, &prop).await?;
    Ok(prop)
}

async fn write_local_proposal(path: &Path, prop: &Proposal) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(prop).map_err(|e| format!("serialize: {e}"))?;
    tokio::fs::write(path, bytes)
        .await
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

fn apply_patch_local(current: &Document, patch: &JsonValue) -> Result<Document, String> {
    let frontmatter_patch = patch.get("frontmatter").cloned();
    let body_replace = patch
        .get("body_replace")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let body_append = patch
        .get("body_append")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    if body_replace.is_some() && body_append.is_some() {
        return Err("patch sets both body_replace and body_append".into());
    }

    // Round-trip current frontmatter through JSON so we can shallow-merge
    // the patch in the same shape `mcp.rs` accepts. Then deserialize back
    // to `Frontmatter` so the version hash matches what `documents.rs`
    // would have produced for a direct write.
    let mut fm_value = serde_json::to_value(&current.frontmatter)
        .map_err(|e| format!("frontmatter to json: {e}"))?;
    if let Some(fm_patch) = frontmatter_patch {
        let JsonValue::Object(fm_patch_map) = fm_patch else {
            return Err("patch.frontmatter must be an object".into());
        };
        let JsonValue::Object(fm_map) = &mut fm_value else {
            return Err("frontmatter is not an object".into());
        };
        for (k, v) in fm_patch_map {
            if v.is_null() {
                fm_map.remove(&k);
            } else {
                fm_map.insert(k, v);
            }
        }
    }
    let new_fm: Frontmatter =
        serde_json::from_value(fm_value).map_err(|e| format!("frontmatter from json: {e}"))?;

    let body = if let Some(replacement) = body_replace {
        replacement
    } else if let Some(suffix) = body_append {
        let mut combined = current.body.clone();
        combined.push_str(&suffix);
        combined
    } else {
        current.body.clone()
    };

    // Stamp `updated` to today so the local doc's metadata reflects the
    // change, mirroring `commands::doc_write`.
    let mut fm = new_fm;
    fm.updated = Some(Utc::now().date_naive());
    if fm.created.is_none() {
        fm.created = Some(Utc::now().date_naive());
    }
    if fm.extras.is_empty() {
        fm.extras = BTreeMap::new();
    }

    Ok(Document {
        frontmatter: fm,
        body,
    })
}

// ---------- remote DTO shim ----------

fn remote_to_dto(p: orchext_sync::Proposal) -> Proposal {
    Proposal {
        id: p.id,
        doc_id: p.doc_id,
        base_version: p.base_version,
        patch: p.patch,
        reason: p.reason,
        status: p.status,
        actor_token_id: p.actor_token_id,
        actor_token_label: p.actor_token_label,
        actor_account_id: p.actor_account_id.map(|u| u.to_string()),
        decided_by: p.decided_by.map(|u| u.to_string()),
        decided_at: p.decided_at,
        decision_note: p.decision_note,
        applied_version: p.applied_version,
        created_at: p.created_at,
    }
}
