//! Mako-compatible grouping projection of the ordered active notification list.
//!
//! This module does not yet physically reorder notification surfaces or apply
//! Mako's built-in hidden/group-index criteria to the rendered view.

use std::collections::{HashMap,HashSet};
use crate::notification::{Notification, Urgency};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupField {
    AppName, AppIcon, Category, DesktopEntry, Summary, Body,
    Urgency, Actionable, Expiring,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroupSpec {
    pub fields: Vec<GroupField>,
    /// Mako's group-by=none disables grouping altogether.
    pub disabled: bool,
}

impl GroupSpec {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut fields = Vec::new();
        let mut disabled = false;
        // Mako tokenizes group-by using strtok_r, ignoring empty tokens.
        for token in text.split(',').filter(|token| !token.is_empty()) {
            let field = match token {
                "app-name" => Some(GroupField::AppName),
                "app-icon" => Some(GroupField::AppIcon),
                "category" => Some(GroupField::Category),
                "desktop-entry" => Some(GroupField::DesktopEntry),
                "summary" => Some(GroupField::Summary),
                "body" => Some(GroupField::Body),
                "urgency" => Some(GroupField::Urgency),
                "actionable" => Some(GroupField::Actionable),
                "expiring" => Some(GroupField::Expiring),
                "none" => { disabled = true; None },
                // Layout-specific grouping fields require a surface association.
                "" | "grouped" | "group-index" | "anchor" | "output" =>
                    return Err(format!("group field not yet supported in projection: {token}")),
                _ => return Err(format!("unknown Mako group field: {token}")),
            };
            if let Some(field) = field {
                if !fields.contains(&field) { fields.push(field); }
            }
        }
        // The 'none' bit short-circuits matching in upstream Mako, even
        // when other valid fields were provided in the same specification.
        if disabled { fields.clear(); }
        Ok(Self { fields, disabled })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// IDs in Mako display order, with the first ID visible by default.
    pub ids: Vec<u32>,
}

impl Group {
    pub fn count(&self) -> usize { self.ids.len() }
    pub fn hidden_by_default(&self) -> usize { self.ids.len().saturating_sub(1) }
    pub fn index_of(&self, id: u32) -> Option<i32> {
        if self.ids.len() < 2 { return self.ids.iter().any(|&x| x == id).then_some(-1); }
        self.ids.iter().position(|&x| x == id).map(|i| i as i32)
    }
}

fn key(note: &Notification, spec: &GroupSpec) -> Vec<String> {
    spec.fields.iter().map(|f| match f {
        GroupField::AppName => note.app_name.clone(),
        GroupField::AppIcon => note.app_icon.clone(),
        GroupField::Category => note.category.clone(),
        GroupField::DesktopEntry => note.desktop_entry.clone(),
        GroupField::Summary => note.summary.clone(),
        GroupField::Body => note.body.clone(),
        GroupField::Urgency => match note.urgency {
            Urgency::Unknown => "unknown",Urgency::Low => "low",
            Urgency::Normal => "normal", Urgency::Critical => "critical"
        }.to_owned(),
        GroupField::Actionable => (!note.actions.is_empty()).to_string(),
        GroupField::Expiring => (note.requested_timeout_ms != 0).to_string(),
    }).collect()
}

/// Project one grouping pass without losing Mako's existing notification order.
///
/// A group is positioned at the first matching notification, and subsequent
/// members appear with the first. Ungrouped notifications stay independent.
pub fn group_ordered<'a>(
    notifications: impl IntoIterator<Item = &'a Notification>,
    spec: &GroupSpec,
) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    let mut positions: HashMap<Vec<String>, usize> = HashMap::new();
    for note in notifications {
        if spec.disabled {
            groups.push(Group {ids: vec![note.id]});
            continue;
        }
        let k = key(note, spec);
        if let Some(&idx) = positions.get(&k) {
            groups[idx].ids.push(note.id);
        } else {
            positions.insert(k, groups.len());
            groups.push(Group {ids: vec![note.id]});
        }
    }
    groups
}

/// Group a display-order snapshot using each leading notification's resolved
/// group-by style rather than assuming one global grouping specification.
/// This is a projection: it does not mutate notification IDs or active order.
pub fn group_ordered_scoped<'a>(
    runtime:&crate::runtime::SharedRuntime,
    notifications:impl IntoIterator<Item=&'a Notification>,
    modes:&[String],
)->Result<Vec<Group>,String>{
    let notes:Vec<&Notification>=notifications.into_iter().collect();
    let mut claimed=HashSet::new();
    let mut groups=Vec::new();
    for leader in &notes {
        if claimed.contains(&leader.id){continue;}
        let style=crate::desktop::style_for_note_index(runtime,leader,modes,-1);
        let spec=GroupSpec::parse(style.get("group-by")
            .map(String::as_str).unwrap_or("none"))?;
        let mut ids=vec![leader.id];
        claimed.insert(leader.id);
        if !spec.disabled {
            let all=group_ordered(notes.iter().copied(),&spec);
            if let Some(matching)=all.into_iter()
                .find(|g|g.ids.contains(&leader.id)) {
                for id in matching.ids {
                    if claimed.insert(id){ids.push(id);}
                }
            }
        }
        groups.push(Group{ids});
    }
    Ok(groups)
}

/// Select the Mako dismissal group using the TARGET notification's
/// resolved group-by setting, not the global setting. This matters for
/// configurations such as [category=warning] group-by=app-name.
pub fn active_group_ids(
    runtime:&crate::runtime::SharedRuntime,target_id:u32
)->Result<Vec<u32>,String>{
    // Clone snapshots before resolving style; callers may subsequently
    // close multiple IDs without retaining notification-store locks.
    let (notifications,modes)={
        let store=runtime.store.lock()
            .map_err(|_|"notification store lock poisoned")?;
        let mode_state=runtime.modes();
        let modes=mode_state.lock()
            .map_err(|_|"mode state lock poisoned")?;
        (store.active().iter().cloned().collect::<Vec<_>>(),modes.clone())
    };
    let Some(target)=notifications.iter().find(|n|n.id==target_id) else {
        return Ok(Vec::new());
    };
    // Mako forbids group-by under grouped/group-index criteria, so the
    // pre-group style resolves the relevant group-by rule without cycling.
    let style=crate::desktop::style_for_note_index(runtime,target,&modes,-1);
    let spec=GroupSpec::parse(style.get("group-by")
        .map(String::as_str).unwrap_or("none"))?;
    Ok(group_ordered(notifications.iter(),&spec).into_iter()
        .find(|g|g.ids.contains(&target_id))
        .map(|g|g.ids).unwrap_or_default())
}

/// Resolve whether a notification is the grouped leader or another member
/// using its own grouping criteria, as on Mako's notification insertion path.
pub fn active_group_index(
    runtime:&crate::runtime::SharedRuntime,target_id:u32
)->Result<i32,String>{
    // A notification can belong to a group created by a different
    // notification's criteria. Its own group-by may still be "none".
    let (notes,modes)={
        let store=runtime.store.lock()
            .map_err(|_|"notification store lock poisoned")?;
        let mode_state=runtime.modes();
        let modes=mode_state.lock()
            .map_err(|_|"mode state lock poisoned")?;
        (store.active().iter().cloned().collect::<Vec<_>>(),modes.clone())
    };
    Ok(group_ordered_scoped(runtime,notes.iter(),&modes)?
        .into_iter().find(|group|group.ids.contains(&target_id))
        .and_then(|group|group.index_of(target_id)).unwrap_or(-1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notification::{NotificationStore, Request};

    fn request(app: &str, category: &str) -> Request {
        Request { app_name: app.into(), category: category.into(), ..Request::default() }
    }

    #[test]
    fn noncontiguous_notification_groups_are_collected_in_first_occurrence_order() {
        let mut state = NotificationStore::new(5);
        let older = state.push(request("alpha", "success"));
        let middle = state.push(request("beta", "warning"));
        let newest = state.push(request("alpha", "failure"));
        let spec = GroupSpec::parse("app-name").unwrap();
        let groups = group_ordered(state.active(), &spec);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].ids, vec![newest, older]);
        assert_eq!(groups[1].ids, vec![middle]);
        assert_eq!(groups[0].hidden_by_default(), 1);
        assert_eq!(groups[0].index_of(newest), Some(0));
        assert_eq!(groups[0].index_of(older), Some(1));
        assert_eq!(groups[1].index_of(middle), Some(-1));
    }

    #[test]
    fn composite_key_and_disabled_groups() {
        let mut state = NotificationStore::new(5);
        state.push(request("same", "warning"));
        state.push(request("same", "success"));
        let split = group_ordered(state.active(), &GroupSpec::parse("app-name,category").unwrap());
        assert_eq!(split.len(), 2);
        let merged = group_ordered(state.active(), &GroupSpec::parse("app-name").unwrap());
        assert_eq!(merged.len(), 1);
        let ungrouped = group_ordered(state.active(), &GroupSpec::parse("none").unwrap());
        assert_eq!(ungrouped.len(), 2);
    }

    #[test]
    fn empty_group_spec_matches_all_notifications_like_mako() {
        let mut state = NotificationStore::new(5);
        state.push(request("alpha", "warning"));
        state.push(request("beta", "success"));
        let grouped = group_ordered(state.active(), &GroupSpec::parse("").unwrap());
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].ids.len(), 2);
        let disabled = group_ordered(state.active(), &GroupSpec::parse("none,app-name").unwrap());
        assert_eq!(disabled.len(), 2);
        assert_eq!(GroupSpec::parse("app-name,,category").unwrap().fields.len(), 2);
    }

    #[test]
    fn scoped_group_rule_applies_only_to_the_selected_target() {
        let runtime=crate::runtime::SharedRuntime::new(
            crate::config::Config::parse(
                "group-by=none\n[category=warning]\ngroup-by=app-name\n"
            ).unwrap()
        );
        let warning=runtime.store.lock().unwrap().push(Request{
            app_name:"sender".into(),category:"warning".into(),
            ..Request::default()
        });
        let regular=runtime.store.lock().unwrap().push(Request{
            app_name:"sender".into(),category:"success".into(),
            ..Request::default()
        });
        let unrelated=runtime.store.lock().unwrap().push(Request{
            app_name:"other".into(),category:"success".into(),
            ..Request::default()
        });
        assert_eq!(active_group_ids(&runtime,warning).unwrap(),
            vec![regular,warning]);
        assert_eq!(active_group_ids(&runtime,regular).unwrap(),vec![regular]);
        assert_eq!(active_group_ids(&runtime,unrelated).unwrap(),vec![unrelated]);
        // Warning arrived before the ungrouped ordinary note; inserting
        // an ordinary note does not retroactively run warning's grouping.
        assert_eq!(active_group_index(&runtime,warning).unwrap(),-1);
        assert_eq!(active_group_index(&runtime,regular).unwrap(),-1);
        assert_eq!(active_group_index(&runtime,unrelated).unwrap(),-1);
    }

    #[test]
    fn unsupported_fields_fail_explicitly() {
        assert!(GroupSpec::parse("app-name,unsupported").is_err());
        assert_eq!(GroupSpec::parse("none,app-name").unwrap(), GroupSpec::parse("none").unwrap());
        assert!(GroupSpec::parse("group-index").is_err());
        assert!(GroupSpec::parse("app-name,unknown").is_err());
    }
}
