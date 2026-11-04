use crate::*;
use std::ops::Bound::{Included, Unbounded};
#[derive(Default)]
pub(crate) struct RouteIndex {
    entries: BTreeMap<SubscriptionId, SubscriptionSpec>,
    exact: BTreeMap<String, BTreeMap<ComponentRef, BTreeSet<SubscriptionId>>>,
    wildcard: BTreeMap<ComponentRef, BTreeMap<SubscriptionId, topic::TopicFilter>>,
}
impl RouteIndex {
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn contains_key(&self, id: &SubscriptionId) -> bool {
        self.entries.contains_key(id)
    }
    pub fn insert(
        &mut self,
        id: SubscriptionId,
        spec: SubscriptionSpec,
        filter: topic::TopicFilter,
    ) {
        if filter.has_wildcards() {
            self.wildcard
                .entry(spec.target.clone())
                .or_default()
                .insert(id, filter);
        } else {
            self.exact
                .entry(spec.topic_pattern.clone())
                .or_default()
                .entry(spec.target.clone())
                .or_default()
                .insert(id);
        }
        self.entries.insert(id, spec);
    }
    pub fn remove(&mut self, id: &SubscriptionId) -> Option<SubscriptionSpec> {
        let spec = self.entries.remove(id)?;
        if let Some(patterns) = self.wildcard.get_mut(&spec.target) {
            patterns.remove(id);
            if patterns.is_empty() {
                self.wildcard.remove(&spec.target);
            }
        }
        if let Some(targets) = self.exact.get_mut(&spec.topic_pattern) {
            if let Some(ids) = targets.get_mut(&spec.target) {
                ids.remove(id);
                if ids.is_empty() {
                    targets.remove(&spec.target);
                }
            }
            if targets.is_empty() {
                self.exact.remove(&spec.topic_pattern);
            }
        }
        Some(spec)
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn retain(&mut self, mut keep: impl FnMut(&SubscriptionId, &SubscriptionSpec) -> bool) {
        let removed: Vec<_> = self
            .entries
            .iter()
            .filter(|(id, s)| !keep(id, s))
            .map(|(id, _)| *id)
            .collect();
        for id in removed {
            self.remove(&id);
        }
    }
    fn targets<'a, T>(
        targets: &'a BTreeMap<ComponentRef, T>,
        destination: &'a Destination,
    ) -> std::collections::btree_map::Range<'a, ComponentRef, T> {
        targets.range::<ComponentRef, _>(match destination {
            Destination::Component(c) => (Included(c), Included(c)),
            _ => (Unbounded, Unbounded),
        })
    }
    pub fn matching<'a>(
        &'a self,
        topic: &'a str,
        destination: &'a Destination,
    ) -> impl Iterator<Item = &'a SubscriptionSpec> {
        let valid = topic::validate_name(topic).is_ok();
        let eligible = move |s: &&SubscriptionSpec| {
            valid
                && match &s.destination_pattern {
                    DestinationPattern::Any => true,
                    DestinationPattern::Exact(d) => d == destination,
                }
        };
        let exact = self
            .exact
            .get(topic)
            .into_iter()
            .flat_map(move |targets| Self::targets(targets, destination))
            .flat_map(|(_, ids)| ids)
            .map(|id| &self.entries[id])
            .filter(eligible);
        let wildcard = Self::targets(&self.wildcard, destination)
            .flat_map(|(_, patterns)| patterns)
            .filter_map(move |(id, filter)| {
                let spec = &self.entries[id];
                if !eligible(&spec) {
                    return None;
                }
                filter.matches_validated(topic).then_some(spec)
            });
        exact.chain(wildcard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mqtt_vectors_agree_with_helper_and_route_index() {
        for &(id, pattern, topic, filter_valid, name_valid, matched) in
            crate::topic_vectors::VECTORS
        {
            assert_eq!(topic_matches(pattern, topic), matched, "helper: {id}");
            assert_eq!(
                topic::validate_name(topic).is_ok(),
                name_valid,
                "name: {id}"
            );
            let filter = topic::TopicFilter::parse(pattern);
            assert_eq!(filter.is_ok(), filter_valid, "filter: {id}");
            if let Ok(filter) = filter {
                let mut index = RouteIndex::default();
                let target = ComponentRef::module("conformance");
                index.insert(1, SubscriptionSpec::new(1, target, pattern), filter);
                assert_eq!(
                    index.matching(topic, &Destination::Broadcast).count(),
                    usize::from(matched),
                    "index: {id}"
                );
                if pattern.contains(':') {
                    assert!(
                        index.wildcard.is_empty(),
                        "literal filter must use exact index"
                    );
                }
                index.remove(&1).unwrap();
                assert!(index.exact.is_empty() && index.wildcard.is_empty());
                assert_eq!(index.matching(topic, &Destination::Broadcast).count(), 0);
            }
        }
    }
}
