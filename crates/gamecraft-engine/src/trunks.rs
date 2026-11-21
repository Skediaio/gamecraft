//! The trunks of a world's trees, as collision data for the simulation.
//!
//! Derived, never stored: every plant entity whose species is a tree, conifer or palm gets a
//! column at its position, sized by the trunk of the very mesh the renderer draws for its
//! species, seed and growth ([`crate::plants::trunk_profile`]) and by the entity's `scale`, so
//! drawing and collision agree. Small plants, grass and shrubs do not block. The columns are
//! handed to the simulation as a [`TrunkSource`], which asks again whenever the running
//! world is replaced; an unchanged set of plants gives the same [`Trunks`] without
//! rebuilding the grid, and a species, seed and growth is generated once.
//!
//! The trunks of the document (while nobody plays) come from the same source
//! ([`crate::Session::current_trunks`]).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use gamecraft_math::StateHasher;
use gamecraft_sim::{Trunk, TrunkSource, Trunks};
use gamecraft_world::World;

use crate::plants::{self, TrunkProfile};

/// Most (species, seed, growth) profiles kept; the cache starts over beyond it.
const MAX_PROFILES: usize = 4096;

type ProfileKey = (String, u64, u64);

#[derive(Debug, Default)]
struct Cache {
    profiles: BTreeMap<ProfileKey, Option<TrunkProfile>>,
    /// The fingerprint of the plants the last answer was made for, and the answer.
    last: Option<(u64, Arc<Trunks>)>,
}

/// The trunks of the plants of a world: the engine's [`TrunkSource`].
#[derive(Debug, Default)]
pub struct PlantTrunks {
    cache: Mutex<Cache>,
}

/// A hash of everything about the plants of `world` that decides their trunks.
fn fingerprint(world: &World) -> u64 {
    let mut h = StateHasher::new();
    for e in world.entities() {
        let Some(p) = e.plant() else { continue };
        h.write_u64(e.id.0);
        h.write_str(&p.species);
        h.write_u64(p.seed);
        h.write_f64(p.growth);
        for v in e.position {
            h.write_f64(v);
        }
        h.write_f64(e.scale);
    }
    h.finish()
}

impl TrunkSource for PlantTrunks {
    fn trunks_of(&self, world: &World) -> Arc<Trunks> {
        let print = fingerprint(world);
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((last, trunks)) = &cache.last
            && *last == print
        {
            return trunks.clone();
        }
        if cache.profiles.len() > MAX_PROFILES {
            cache.profiles.clear();
        }
        let mut list = Vec::new();
        for e in world.entities() {
            let Some(p) = e.plant() else { continue };
            let key = (p.species.clone(), p.seed, p.growth.to_bits());
            let profile = *cache
                .profiles
                .entry(key)
                .or_insert_with(|| plants::trunk_profile(&p.species, p.seed, p.growth));
            let Some(profile) = profile else { continue };
            list.push(Trunk {
                entity: e.id,
                position: e.position,
                radius_m: profile.radius_m * e.scale,
                height_m: profile.height_m * e.scale,
            });
        }
        let trunks = Arc::new(Trunks::new(list));
        cache.last = Some((print, trunks.clone()));
        trunks
    }
}
