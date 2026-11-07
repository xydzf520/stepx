//! Account-bound identities of desktop-owned inference sessions. These are not
//! user discussions; titles/content are never used to decide ownership.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeSet, fs, io::Write, path::{Path, PathBuf}};

#[derive(Serialize, Deserialize)]
struct OwnedSession { schema:u32, scope_id:String, device:String, account:String, session_key:String }

pub fn default_context_root()->PathBuf { crate::resources::user_state().join("stepx-companion/context") }

fn check_dir(path:&Path)->Result<()> {
    let metadata=fs::symlink_metadata(path)?;
    ensure!(metadata.is_dir()&&!metadata.file_type().is_symlink(),"推理会话登记目录无效");Ok(())
}
fn private_dir(path:&Path)->Result<()> {
    match fs::create_dir(path) {
        Ok(())=>{},
        Err(error) if error.kind()==std::io::ErrorKind::AlreadyExists=>{},
        Err(error)=>return Err(error.into()),
    }
    check_dir(path)?;
    #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;fs::set_permissions(path,fs::Permissions::from_mode(0o700))?;}
    Ok(())
}
fn key_path(dir:&Path,key:&str)->PathBuf {
    dir.join(format!("{}.json",crate::context_ledger::scope_id(key,Some("owned-inference-session-v1"))))
}
fn read_json(path:&Path)->Result<Value> {
    let metadata=fs::symlink_metadata(path)?;
    ensure!(metadata.is_file()&&!metadata.file_type().is_symlink()&&metadata.len()<=16*1024*1024,"推理会话登记文件无效");
    serde_json::from_slice(&fs::read(path)?).context("推理会话登记内容无效")
}
/// Persist before sending any prompt. One immutable file per identity avoids
/// lost updates across processes and survives clearing the document scope.
pub fn register(root:&Path,device:&str,account:&str,key:&str)->Result<()> {
    ensure!(!device.is_empty()&&!account.is_empty()&&!key.trim().is_empty()&&key.len()<=512,"推理会话身份不完整");
    if !root.exists() {fs::create_dir_all(root)?;}
    check_dir(root)?;
    let registry=root.join(".inference-sessions");private_dir(&registry)?;
    let scope=crate::context_ledger::scope_id(device,Some(account));
    let dir=registry.join(&scope);private_dir(&dir)?;
    let path=key_path(&dir,key);
    let record=OwnedSession {schema:1,scope_id:scope,device:device.into(),account:account.into(),session_key:key.into()};
    if path.exists() {
        let old:OwnedSession=serde_json::from_value(read_json(&path)?)?;
        ensure!(old.schema==1&&old.scope_id==record.scope_id&&old.device==device&&old.account==account&&old.session_key==key,"推理会话登记身份不匹配");
        return Ok(());
    }
    let mut temp=tempfile::NamedTempFile::new_in(&dir)?;
    temp.write_all(&serde_json::to_vec(&record)?)?;temp.as_file().sync_all()?;
    match temp.persist_noclobber(&path) {
        Ok(_)=>Ok(()),
        Err(error) if error.error.kind()==std::io::ErrorKind::AlreadyExists=>register(root,device,account,key),
        Err(error)=>Err(error.error.into()),
    }
}

fn metadata_key(meta:&Value,account:&str)->Option<String> {
    (meta["source"]=="phone_account"&&meta["account_scope"]==account)
        .then(||meta["temporary_session_key"].as_str()).flatten()
        .filter(|key|!key.trim().is_empty()&&key.len()<=512).map(str::to_owned)
}
/// Read only the exact device/account. Older jobs retain inference metadata in
/// every attempt, including rejected/retired drafts. No prompt/title heuristic.
pub fn known(root:&Path,device:&str,account:Option<&str>)->Result<BTreeSet<String>> {
    let mut result=BTreeSet::new();
    let Some(account)=account.filter(|account|!account.is_empty()) else{return Ok(result);};
    if !root.exists() {return Ok(result);}
    check_dir(root)?;
    let scope=crate::context_ledger::scope_id(device,Some(account));
    let registry=root.join(".inference-sessions");
    if registry.exists() {
        check_dir(&registry)?;
        let dir=registry.join(&scope);
        if dir.exists() {
            check_dir(&dir)?;
            for entry in fs::read_dir(&dir)? {
                let path=entry?.path();
                if path.extension().and_then(|ext|ext.to_str())!=Some("json") {continue;}
                let record:OwnedSession=serde_json::from_value(read_json(&path)?)?;
                ensure!(record.schema==1&&record.scope_id==scope&&record.device==device&&record.account==account
                    &&path==key_path(&dir,&record.session_key),"推理会话登记身份不匹配");
                result.insert(record.session_key);
            }
        }
    }
    let scope_dir=root.join(&scope);
    if !scope_dir.exists() {return Ok(result);}
    check_dir(&scope_dir)?;
    let pipeline=scope_dir.join("pipeline");
    if !pipeline.exists() {return Ok(result);}
    check_dir(&pipeline)?;
    for entry in fs::read_dir(&pipeline)? {
        let path=entry?.path();
        if path.extension().and_then(|ext|ext.to_str())!=Some("json") {continue;}
        // Old jobs are independent recovery units. A corrupt, linked or
        // foreign job is not ownership evidence and must not block loading
        // unrelated user sources. The owned registry above stays strict.
        let Ok(job)=read_json(&path) else {continue;};
        if job["scope_id"]!=scope {continue;}
        if let Some(key)=metadata_key(&job["accepted_inference"],account) {result.insert(key);}
        if let Some(attempts)=job["attempts"].as_array() {
            for attempt in attempts {
                if let Some(key)=metadata_key(&attempt["inference"],account) {result.insert(key);}
                // Compatibility with an earlier serialized metadata field.
                if let Some(key)=metadata_key(&attempt["model"],account) {result.insert(key);}
                if let Some(meta)=attempt["model"].as_str().and_then(|text|serde_json::from_str::<Value>(text).ok()) {
                    if let Some(key)=metadata_key(&meta,account) {result.insert(key);}
                }
            }
        }
    }
    // Migrate identities outside the disposable document scope. Clearing old
    // documents must not turn a residual phone title back into user evidence.
    for key in &result {register(root,device,account,key)?;}
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_is_durable_account_bound_and_not_a_desktop_key_prefix() {
        let root=tempfile::tempdir().unwrap();let key="agent:main:desktop-owned";
        register(root.path(),"device","account",key).unwrap();register(root.path(),"device","account",key).unwrap();
        assert_eq!(known(root.path(),"device",Some("account")).unwrap(),BTreeSet::from([key.to_string()]));
        assert!(known(root.path(),"other-device",Some("account")).unwrap().is_empty());
        assert!(known(root.path(),"device",Some("other-account")).unwrap().is_empty());
        assert!(!known(root.path(),"device",Some("account")).unwrap().contains("agent:main:desktop-real-user"));
    }
    #[test]
    fn concurrent_first_registration_keeps_shared_and_distinct_session_identities() {
        let root=tempfile::tempdir().unwrap();
        let barrier=std::sync::Arc::new(std::sync::Barrier::new(12));
        std::thread::scope(|threads| {
            let mut workers=Vec::new();
            for index in 0..12 {
                let barrier=barrier.clone();let path=root.path();
                workers.push(threads.spawn(move || {
                    barrier.wait();
                    register(path,"device","account","shared-session")?;
                    register(path,"device","account",&format!("session-{index}"))
                }));
            }
            for worker in workers {worker.join().unwrap().unwrap();}
        });
        let keys=known(root.path(),"device",Some("account")).unwrap();
        assert_eq!(keys.len(),13);assert!(keys.contains("shared-session"));
        for index in 0..12 {assert!(keys.contains(&format!("session-{index}")));}
        assert!(known(root.path(),"device",Some("other-account")).unwrap().is_empty());
    }
    #[test]
    fn historical_failed_attempt_and_accepted_metadata_migrate_only_matching_phone_account() {
        let root=tempfile::tempdir().unwrap();let scope=crate::context_ledger::scope_id("device",Some("account"));
        let dir=root.path().join(&scope).join("pipeline");fs::create_dir_all(&dir).unwrap();
        let meta=|source:&str,account:&str,key:&str|serde_json::json!({"source":source,"account_scope":account,"temporary_session_key":key});
        let job=serde_json::json!({"scope_id":scope,"attempts":[
            {"error":"old rejected draft","inference":meta("phone_account","account","owned-rejected")},
            {"inference":meta("phone_account","other","foreign-account")},
            {"inference":meta("independent_api","account","foreign-api")},
            {"model":meta("phone_account","account","owned-legacy").to_string()}],
            "accepted_inference":meta("phone_account","account","owned-accepted")});
        fs::write(dir.join("job.json"),serde_json::to_vec(&job).unwrap()).unwrap();
        assert_eq!(known(root.path(),"device",Some("account")).unwrap(),BTreeSet::from(["owned-rejected".into(),"owned-legacy".into(),"owned-accepted".into()]));
        fs::remove_dir_all(root.path().join(&scope)).unwrap();
        assert_eq!(known(root.path(),"device",Some("account")).unwrap(),BTreeSet::from(["owned-rejected".into(),"owned-legacy".into(),"owned-accepted".into()]));
    }
    #[test]
    fn corrupt_and_foreign_legacy_jobs_remain_untouched_and_do_not_block_valid_migration() {
        let root=tempfile::tempdir().unwrap();let scope=crate::context_ledger::scope_id("device",Some("account"));
        let dir=root.path().join(&scope).join("pipeline");fs::create_dir_all(&dir).unwrap();
        let meta=|key:&str|serde_json::json!({"source":"phone_account","account_scope":"account","temporary_session_key":key});
        let valid=serde_json::json!({"scope_id":scope,"attempts":[{"inference":meta("trusted-owned")}]}).to_string();
        let foreign=serde_json::json!({"scope_id":"other-device-account-scope","accepted_inference":meta("must-remain-user-source")}).to_string();
        let corrupt="{\"scope_id\": invalid legacy content";
        fs::write(dir.join("valid.json"),&valid).unwrap();
        fs::write(dir.join("foreign.json"),&foreign).unwrap();
        fs::write(dir.join("corrupt.json"),corrupt).unwrap();
        assert_eq!(known(root.path(),"device",Some("account")).unwrap(),BTreeSet::from(["trusted-owned".into()]));
        assert_eq!(fs::read_to_string(dir.join("valid.json")).unwrap(),valid);
        assert_eq!(fs::read_to_string(dir.join("foreign.json")).unwrap(),foreign);
        assert_eq!(fs::read_to_string(dir.join("corrupt.json")).unwrap(),corrupt);
        fs::remove_dir_all(root.path().join(&scope)).unwrap();
        assert_eq!(known(root.path(),"device",Some("account")).unwrap(),BTreeSet::from(["trusted-owned".into()]));
    }
    #[cfg(unix)]
    #[test]
    fn symlink_registry_or_foreign_scope_cannot_hide_user_discussions() {
        let root=tempfile::tempdir().unwrap();let elsewhere=tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(),root.path().join(".inference-sessions")).unwrap();
        assert!(known(root.path(),"device",Some("account")).is_err());
        assert!(register(root.path(),"device","account","owned").is_err());
        assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(),0);
    }
}
