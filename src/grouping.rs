//! Mako-compatible grouping projection of the ordered active notification list.
//!
//! This module does not yet physically reorder notification surfaces or apply
//! Mako's built-in hidden/group-index criteria to the rendered view.

use std::collections::HashMap;
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
        for token in text.split(',') {
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
        if disabled && !fields.is_empty() {
            return Err("group-by=none cannot be combined with other fields".into());
        }
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
            Urgency::Low => "low", Urgency::Normal => "normal", Urgency::Critical => "critical"
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
        if spec.disabled || spec.fields.is_empty() {
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
    fn unsupported_fields_fail_explicitly() {
        assert!(GroupSpec::parse("app-name,unsupported").is_err());
        assert!(GroupSpec::parse("none,app-name").is_err());
        assert!(GroupSpec::parse("group-index").is_err());
        assert!(GroupSpec::parse("app-name,,category").is_err());
    }
}
