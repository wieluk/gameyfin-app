//! Map a Gameyfin game onto a Ludusavi manifest entry. Anything less than certain comes back as
//! [`TitleMatch::Ambiguous`], since a wrong match restores one game's save over another's.

use crate::error::SaveResult;
use crate::ludusavi::{GameQuery, Ludusavi};

/// Minimum fuzzy score worth offering. On Ludusavi 0.31 this keeps "Celeste" at 0.85 and drops
/// "Aces and Adventures" at 0.75.
const FUZZY_FLOOR: f64 = 0.80;

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub title: String,
    /// `None` for exact/ID matches, which are certain.
    pub score: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TitleMatch {
    /// Identified by store ID or an exact manifest title. Safe to use unattended.
    Certain(String),
    /// Plausible, but a human should confirm before saves are written anywhere.
    Ambiguous(Vec<Candidate>),
    /// The manifest has nothing for this game; it needs a custom-game entry.
    None,
}

impl TitleMatch {
    /// The title to use without asking the user, if there is one.
    pub fn unattended(&self) -> Option<&str> {
        match self {
            TitleMatch::Certain(title) => Some(title),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct GameIdentity {
    pub title: String,
    /// From `GameMetadata.originalIds["steam"]`, when the server exposes it.
    pub steam_app_id: Option<u32>,
    pub gog_id: Option<u64>,
}

pub async fn resolve(lud: &Ludusavi, identity: &GameIdentity) -> SaveResult<TitleMatch> {
    // 1. Store IDs are exact.
    if let Some(app_id) = identity.steam_app_id {
        if let Some(title) = single_title(lud, &GameQuery::SteamId(app_id)).await? {
            return Ok(TitleMatch::Certain(title));
        }
    }
    if let Some(gog_id) = identity.gog_id {
        if let Some(title) = single_title(lud, &GameQuery::GogId(gog_id)).await? {
            return Ok(TitleMatch::Certain(title));
        }
    }

    // 2. Exact title. Ludusavi resolves aliases here too.
    let exact = lud
        .find(&GameQuery::Title {
            title: identity.title.clone(),
            normalized: false,
            fuzzy: false,
        })
        .await?;
    if let Some((title, _)) = exact.games.iter().next() {
        if exact.games.len() == 1 {
            return Ok(TitleMatch::Certain(title.clone()));
        }
    }

    // 3. Normalized: forgiving about edition suffixes, punctuation and year markers.
    // A single hit is still certain, because normalization is deterministic.
    let normalized = lud
        .find(&GameQuery::Title {
            title: identity.title.clone(),
            normalized: true,
            fuzzy: false,
        })
        .await?;
    if normalized.games.len() == 1 {
        let title = normalized.games.keys().next().expect("len checked").clone();
        return Ok(TitleMatch::Certain(title));
    }
    if normalized.games.len() > 1 {
        return Ok(TitleMatch::Ambiguous(candidates(&normalized)));
    }

    // 4. Fuzzy, as a last resort. Never accepted unattended, however high the score:
    // a confident-looking wrong match is exactly the failure mode to avoid.
    let fuzzy = lud
        .find(&GameQuery::Title {
            title: identity.title.clone(),
            normalized: false,
            fuzzy: true,
        })
        .await?;

    let mut found = candidates(&fuzzy);
    found.retain(|c| c.score.is_none_or(|s| s >= FUZZY_FLOOR));
    if found.is_empty() {
        return Ok(TitleMatch::None);
    }
    Ok(TitleMatch::Ambiguous(found))
}

/// A lookup that yields exactly one title, or nothing.
async fn single_title(lud: &Ludusavi, query: &GameQuery) -> SaveResult<Option<String>> {
    let out = lud.find(query).await?;
    let mut keys = out.games.into_keys();
    match (keys.next(), keys.next()) {
        (Some(title), None) => Ok(Some(title)),
        _ => Ok(None),
    }
}

fn candidates(out: &crate::api::ApiOutput<crate::api::FoundGame>) -> Vec<Candidate> {
    let mut list: Vec<Candidate> = out
        .games
        .iter()
        .map(|(title, game)| Candidate {
            title: title.clone(),
            score: game.score,
        })
        .collect();
    // Best first. Scoreless (exact) entries outrank scored ones.
    list.sort_by(|a, b| {
        b.score
            .unwrap_or(f64::INFINITY)
            .partial_cmp(&a.score.unwrap_or(f64::INFINITY))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{CommandOutput, CommandRunner, FakeRunner};
    use std::sync::Arc;

    struct Shared(Arc<FakeRunner>);
    #[async_trait::async_trait]
    impl CommandRunner for Shared {
        async fn run(&self, program: &str, args: &[String]) -> SaveResult<CommandOutput> {
            self.0.run(program, args).await
        }
    }

    fn lud(responses: Vec<CommandOutput>) -> (Ludusavi, Arc<FakeRunner>) {
        let runner = Arc::new(FakeRunner::new(responses));
        (
            Ludusavi::with_runner("/l", "/cfg", Box::new(Shared(runner.clone()))),
            runner,
        )
    }

    fn identity(title: &str) -> GameIdentity {
        GameIdentity {
            title: title.into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn steam_id_short_circuits_everything() {
        let (l, runner) = lud(vec![FakeRunner::ok(r#"{"games":{"Celeste":{}}}"#)]);
        let id = GameIdentity {
            title: "Celeste (2018)".into(),
            steam_app_id: Some(504230),
            gog_id: None,
        };

        let m = resolve(&l, &id).await.unwrap();
        assert_eq!(m, TitleMatch::Certain("Celeste".into()));
        // Exactly one lookup: no title searches were needed.
        assert_eq!(runner.call_count(), 1);
        assert!(runner.call(0).contains(&"--steam-id".to_string()));
    }

    #[tokio::test]
    async fn falls_through_to_exact_title_when_no_ids() {
        let (l, runner) = lud(vec![FakeRunner::ok(r#"{"games":{"Hades":{}}}"#)]);
        let m = resolve(&l, &identity("Hades")).await.unwrap();
        assert_eq!(m, TitleMatch::Certain("Hades".into()));
        assert_eq!(runner.call_count(), 1);
    }

    #[tokio::test]
    async fn single_normalized_hit_is_certain() {
        let (l, runner) = lud(vec![
            FakeRunner::ok(r#"{"games":{}}"#),                 // exact: miss
            FakeRunner::ok(r#"{"games":{"Outer Wilds":{}}}"#), // normalized: one hit
        ]);
        let m = resolve(&l, &identity("outer wilds")).await.unwrap();
        assert_eq!(m, TitleMatch::Certain("Outer Wilds".into()));
        assert!(runner.call(1).contains(&"--normalized".to_string()));
    }

    #[tokio::test]
    async fn multiple_normalized_hits_need_confirmation() {
        let (l, _) = lud(vec![
            FakeRunner::ok(r#"{"games":{}}"#),
            FakeRunner::ok(r#"{"games":{"Doom":{},"Doom II":{}}}"#),
        ]);
        let m = resolve(&l, &identity("doom")).await.unwrap();
        match m {
            TitleMatch::Ambiguous(c) => assert_eq!(c.len(), 2),
            other => panic!("expected ambiguous, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn high_fuzzy_score_is_still_never_accepted_unattended() {
        let (l, _) = lud(vec![
            FakeRunner::ok(r#"{"games":{}}"#),
            FakeRunner::ok(r#"{"games":{}}"#),
            FakeRunner::ok(r#"{"games":{"Celeste":{"score":0.98}}}"#),
        ]);
        let m = resolve(&l, &identity("Celest")).await.unwrap();
        // A lone fuzzy hit is still confirmed with the user.
        assert!(matches!(m, TitleMatch::Ambiguous(_)));
        assert_eq!(m.unattended(), None);
    }

    #[tokio::test]
    async fn weak_fuzzy_candidates_are_discarded() {
        let (l, _) = lud(vec![
            FakeRunner::ok(r#"{"games":{}}"#),
            FakeRunner::ok(r#"{"games":{}}"#),
            FakeRunner::ok(r#"{"games":{"Something Else":{"score":0.31}}}"#),
        ]);
        let m = resolve(&l, &identity("Totally Unknown Game"))
            .await
            .unwrap();
        assert_eq!(m, TitleMatch::None);
    }

    #[tokio::test]
    async fn candidates_are_ranked_best_first() {
        let (l, _) = lud(vec![
            FakeRunner::ok(r#"{"games":{}}"#),
            FakeRunner::ok(r#"{"games":{}}"#),
            FakeRunner::ok(r#"{"games":{"Far":{"score":0.83},"Near":{"score":0.95}}}"#),
        ]);
        match resolve(&l, &identity("nearish")).await.unwrap() {
            TitleMatch::Ambiguous(c) => {
                assert_eq!(c[0].title, "Near");
                assert_eq!(c[1].title, "Far");
            }
            other => panic!("expected ambiguous, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fuzzy_floor_drops_noise_but_keeps_real_near_misses() {
        // Real Ludusavi 0.31 output for the query "celest".
        let (l, _) = lud(vec![
            FakeRunner::ok(r#"{"games":{}}"#),
            FakeRunner::ok(r#"{"games":{}}"#),
            FakeRunner::ok(
                r#"{"games":{"Aces and Adventures":{"score":0.75},"Celeste":{"score":0.8492},"Celestial":{"score":0.7963}}}"#,
            ),
        ]);
        match resolve(&l, &identity("celest")).await.unwrap() {
            TitleMatch::Ambiguous(c) => {
                let titles: Vec<_> = c.iter().map(|x| x.title.as_str()).collect();
                assert_eq!(titles, vec!["Celeste"]);
            }
            other => panic!("expected ambiguous, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn steam_id_matching_nothing_falls_through() {
        let (l, runner) = lud(vec![
            FakeRunner::ok(r#"{"games":{}}"#),                // steam id: miss
            FakeRunner::ok(r#"{"games":{"Indie Game":{}}}"#), // exact title: hit
        ]);
        let id = GameIdentity {
            title: "Indie Game".into(),
            steam_app_id: Some(999999),
            gog_id: None,
        };
        let m = resolve(&l, &id).await.unwrap();
        assert_eq!(m, TitleMatch::Certain("Indie Game".into()));
        assert_eq!(runner.call_count(), 2);
    }
}
