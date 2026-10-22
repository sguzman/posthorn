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

/// Match the target's incoming group-by criteria against an active note.
pub fn same_group(note:&Notification,target:&Notification,spec:&GroupSpec)->bool{
    !spec.disabled && key(note,spec)==key(target,spec)
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

/// Replay grouping passes in notification arrival order, as Mako does on
/// each Notify. A newest-first or oldest-first visual sort does *not* change
/// the order in which per-notification group-by rules are executed.
///
/// This remains a projection of currently active IDs: a replaced ID keeps
/// its original chronological identity, and full replacement-time regrouping
/// is pending. The active store is never mutated by rendering.
pub fn group_ordered_scoped<'a>(
    runtime:&crate::runtime::SharedRuntime,
    notifications:impl IntoIterator<Item=&'a Notification>,
    modes:&[String],
)->Result<Vec<Group>,String>{
    let notes:Vec<&Notification>=notifications.into_iter().collect();
    let by_id:HashMap<u32,&Notification>=notes.iter().map(|n|(n.id,*n)).collect();
    // When notifications have gone through their actual Notify lifecycle,
    // consume the group indices retained by the store. On close, Mako can
    // regroup surviving members even if no survivor's group-by would
    // independently create that group. Replay would discard this state.
    if notes.iter().all(|n|n.grouping_applied) {
        let mut recorded:Vec<Group>=Vec::new();
        for note in &notes {
            let index=note.group_index;
            let continuing=index>0 && recorded.last().is_some_and(|group|
                group.ids.len()==index as usize
                && group.ids.first().is_some_and(|first|
                    by_id.get(first).is_some_and(|n|n.group_index==0)));
            if continuing {
                if let Some(last)=recorded.last_mut(){last.ids.push(note.id);}
            }else{
                recorded.push(Group{ids:vec![note.id]});
            }
        }
        return Ok(recorded);
    }
    let mut order:Vec<u32>=notes.iter().map(|n|n.id).collect();
    let mut group_index:HashMap<u32,i32>=order.iter().map(|id|(*id,-1)).collect();
    // Fresh Mako notification IDs increase monotonically. Replay the
    // arrival-triggered grouping passes, not the final display order.
    let mut arrivals=notes.clone();
    arrivals.sort_by_key(|n|n.id);
    for arriving in arrivals {
        let style=crate::desktop::style_for_note_index(runtime,arriving,modes,-1);
        let spec=GroupSpec::parse(style.get("group-by")
            .map(String::as_str).unwrap_or("none"))?;
        if spec.disabled{continue;}
        let wanted=key(arriving,&spec);
        let matched:Vec<u32>=order.iter().copied().filter(|id|{
            by_id.get(id).is_some_and(|note|
                note.id<=arriving.id && key(note,&spec)==wanted)
        }).collect();
        if matched.is_empty(){continue;}
        let selected:HashSet<u32>=matched.iter().copied().collect();
        let first=order.iter().position(|id|selected.contains(id))
            .ok_or("group projection lost an active notification")?;
        // Upstream group_notifications extracts all matched members from
        // the ordered list and inserts them at their first previous position.
        order.retain(|id|!selected.contains(id));
        order.splice(first..first,matched.iter().copied());
        for (index,id) in matched.iter().enumerate() {
            group_index.insert(*id,if matched.len()==1{-1}else{index as i32});
        }
    }
    let mut groups:Vec<Group>=Vec::new();
    for id in order {
        let index=group_index.get(&id).copied().unwrap_or(-1);
        if index>0 && groups.last().is_some_and(|g|
            g.ids.len()==index as usize
                && g.ids.first().is_some_and(|head|
                    group_index.get(head)==Some(&0))) {
            groups.last_mut().expect("checked last group").ids.push(id);
        } else {
            groups.push(Group{ids:vec![id]});
        }
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
    fn scoped_arrival_grouping_is_independent_of_time_sort_direction() {
        for ascending in [true,false] {
            let runtime=crate::runtime::SharedRuntime::new(
                crate::config::Config::parse(&format!(
                    "sort={}time\\ngroup-by=none\\n[category=warning]\\ngroup-by=app-name\\n",
                    if ascending{"+"}else{"-"}
                ).replace("\\n","\n")).unwrap()
            );
            let old=runtime.store.lock().unwrap().push(request("same","success"));
            let middle=runtime.store.lock().unwrap().push(request("other","success"));
            let warning=runtime.store.lock().unwrap().push(request("same","warning"));
            let active=runtime.store.lock().unwrap();
            let groups=group_ordered_scoped(&runtime,active.active(),
                &["default".into()]).unwrap();
            assert_eq!(groups.len(),2);
            assert_eq!(groups[0].ids,
                if ascending{vec![old,warning]}else{vec![warning,old]});
            assert_eq!(groups[1].ids,vec![middle]);
        }
    }

    #[test]
    fn scoped_rules_only_see_notifications_that_have_already_arrived() {
        let runtime=crate::runtime::SharedRuntime::new(
            crate::config::Config::parse(
                "sort=+time\ngroup-by=none\n[category=warning]\ngroup-by=app-name\n"
            ).unwrap()
        );
        let warning=runtime.store.lock().unwrap().push(request("same","warning"));
        let later=runtime.store.lock().unwrap().push(request("same","success"));
        let groups=group_ordered_scoped(&runtime,
            runtime.store.lock().unwrap().active().iter().cloned().collect::<Vec<_>>().iter(),
            &["default".into()]).unwrap();
        assert_eq!(groups,vec![Group{ids:vec![warning]},Group{ids:vec![later]}]);
    }

    #[test]
    fn unsupported_fields_fail_explicitly() {
        assert!(GroupSpec::parse("app-name,unsupported").is_err());
        assert_eq!(GroupSpec::parse("none,app-name").unwrap(), GroupSpec::parse("none").unwrap());
        assert!(GroupSpec::parse("group-index").is_err());
        assert!(GroupSpec::parse("app-name,unknown").is_err());
    }
}
