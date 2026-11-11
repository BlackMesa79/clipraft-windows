use super::Storage;
use anyhow::{bail, Result};
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct GroupView {
    pub id: i64,
    pub name: String,
    pub sort: i64,
    pub count: usize,
}

fn group_exists(conn: &Connection, id: i64) -> Result<()> {
    if !conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM groups WHERE id=?1)",
        [id],
        |r| r.get::<_, bool>(0),
    )? {
        bail!("分组不存在");
    }
    Ok(())
}

fn validated_name(conn: &Connection, name: &str, except: Option<i64>) -> Result<String> {
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}'))
    {
        bail!("组名不能包含控制字符或零宽字符");
    }
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if !(1..=40).contains(&name.chars().count()) {
        bail!("组名去掉首尾空白后必须为 1 到 40 个字符");
    }
    // SQLite NOCASE 仅处理 ASCII；这里统一使用 Rust Unicode 小写转换。
    let folded = name.to_lowercase();
    let mut stmt = conn.prepare("SELECT id,name FROM groups")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (id, existing) = row?;
        if Some(id) != except
            && existing
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
                == folded
        {
            bail!("已存在同名分组（不区分大小写）");
        }
    }
    Ok(name)
}

impl Storage {
    pub fn list_groups(&self) -> Result<Vec<GroupView>> {
        let mut stmt = self.conn.prepare("SELECT g.id,g.name,g.sort,count(i.id) FROM groups g LEFT JOIN items i ON i.group_id=g.id GROUP BY g.id ORDER BY g.sort,g.id")?;
        let rows = stmt.query_map([], |r| {
            Ok(GroupView {
                id: r.get(0)?,
                name: r.get(1)?,
                sort: r.get(2)?,
                count: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn create_group(&mut self, name: &str) -> Result<GroupView> {
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let name = validated_name(&tx, name, None)?;
        let count: i64 = tx.query_row("SELECT count(*) FROM groups", [], |r| r.get(0))?;
        if count >= 100 {
            bail!("分组数量不能超过 100 个");
        }
        let sort: i64 = tx.query_row("SELECT coalesce(max(sort),-1)+1 FROM groups", [], |r| {
            r.get(0)
        })?;
        tx.execute(
            "INSERT INTO groups(name,sort,created) VALUES(?1,?2,?3)",
            params![name, sort, super::unix_millis()],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(GroupView {
            id,
            name,
            sort,
            count: 0,
        })
    }

    pub fn rename_group(&mut self, id: i64, name: &str) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        group_exists(&tx, id)?;
        let name = validated_name(&tx, name, Some(id))?;
        tx.execute("UPDATE groups SET name=?2 WHERE id=?1", params![id, name])?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete_group(&mut self, id: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        group_exists(&tx, id)?;
        tx.execute("UPDATE items SET group_id=NULL WHERE group_id=?1", [id])?;
        tx.execute("DELETE FROM groups WHERE id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn move_to_group(&mut self, ids: &[String], group_id: Option<i64>) -> Result<usize> {
        let tx = self.conn.transaction()?;
        if let Some(id) = group_id {
            group_exists(&tx, id)?;
        }
        let mut changed = 0;
        {
            let mut stmt =
                tx.prepare("UPDATE items SET group_id=?2,expires_at=CASE WHEN ?2 IS NOT NULL AND sensitive=1 THEN NULL ELSE expires_at END WHERE id=?1 AND (group_id IS NOT ?2 OR (?2 IS NOT NULL AND sensitive=1 AND expires_at IS NOT NULL))")?;
            for id in ids {
                changed += stmt.execute(params![id, group_id])?;
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    pub fn set_pinned(&mut self, ids: &[String], value: bool) -> Result<usize> {
        self.set_flag(
            ids,
            value,
            "UPDATE items SET pinned=?2,expires_at=CASE WHEN ?2 AND sensitive=1 THEN NULL ELSE expires_at END WHERE id=?1 AND (pinned<>?2 OR (?2 AND sensitive=1 AND expires_at IS NOT NULL))",
        )
    }
    pub fn set_favorite(&mut self, ids: &[String], value: bool) -> Result<usize> {
        self.set_flag(
            ids,
            value,
            "UPDATE items SET favorite=?2,expires_at=CASE WHEN ?2 AND sensitive=1 THEN NULL ELSE expires_at END WHERE id=?1 AND (favorite<>?2 OR (?2 AND sensitive=1 AND expires_at IS NOT NULL))",
        )
    }
    fn set_flag(&mut self, ids: &[String], value: bool, sql: &str) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut changed = 0;
        {
            let mut stmt = tx.prepare(sql)?;
            for id in ids {
                changed += stmt.execute(params![id, value])?;
            }
        }
        tx.commit()?;
        Ok(changed)
    }
}
