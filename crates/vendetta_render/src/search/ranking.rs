use std::cmp::Ordering;

pub fn tokenize_search_text(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect()
}

pub fn score_search_query(query: &str, entry_text: &str, entry_tokens: &[String]) -> Option<u32> {
    let query_trim = query.trim().to_lowercase();
    if query_trim.is_empty() {
        return None;
    }

    let query_tokens = tokenize_search_text(&query_trim);
    if query_tokens.is_empty() {
        return None;
    }

    let text_lower = entry_text.to_lowercase();
    let mut matched_tokens = 0;
    let mut exact_token_matches = 0;

    for q_token in &query_tokens {
        let mut found = false;
        for e_token in entry_tokens {
            if e_token == q_token {
                exact_token_matches += 1;
                found = true;
                break;
            } else if e_token.starts_with(q_token) {
                found = true;
                break;
            }
        }
        if found {
            matched_tokens += 1;
        }
    }

    if matched_tokens == query_tokens.len() {
        if text_lower.contains(&query_trim) {
            Some(100)
        } else if exact_token_matches == query_tokens.len() {
            Some(50)
        } else {
            Some(20 + (exact_token_matches as u32 * 5))
        }
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
pub fn compare_search_results(
    score_a: u32,
    date_a: i64,
    peer_a: i64,
    msg_a: i64,
    score_b: u32,
    date_b: i64,
    peer_b: i64,
    msg_b: i64,
) -> Ordering {
    score_b
        .cmp(&score_a)
        .then_with(|| date_b.cmp(&date_a))
        .then_with(|| peer_a.cmp(&peer_b))
        .then_with(|| msg_a.cmp(&msg_b))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedSearchResult {
    pub score: u32,
    pub date: i64,
    pub peer_id: i64,
    pub msg_id: i64,
    pub entry_id: String,
}

impl BoundedSearchResult {
    pub fn compare(&self, other: &Self) -> Ordering {
        compare_search_results(
            self.score,
            self.date,
            self.peer_id,
            self.msg_id,
            other.score,
            other.date,
            other.peer_id,
            other.msg_id,
        )
    }
}

#[derive(Debug, Clone)]
pub struct BoundedTopResults {
    limit: usize,
    results: Vec<BoundedSearchResult>,
}

impl BoundedTopResults {
    pub fn new(limit: usize) -> Self {
        Self {
            limit: limit.max(1),
            results: Vec::with_capacity(limit + 1),
        }
    }

    pub fn insert(&mut self, item: BoundedSearchResult) {
        if self.results.len() >= self.limit
            && let Some(worst) = self.results.last()
            && item.compare(worst) != Ordering::Less
        {
            return;
        }

        let idx = self
            .results
            .partition_point(|other| other.compare(&item) != Ordering::Greater);

        self.results.insert(idx, item);
        if self.results.len() > self.limit {
            self.results.pop();
        }
    }

    pub fn results(&self) -> &[BoundedSearchResult] {
        &self.results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranking_scores_exact_phrase_and_tokens() {
        let text = "Hello Telegram Archive World!";
        let tokens = tokenize_search_text(text);

        assert_eq!(
            score_search_query("Telegram Archive", text, &tokens),
            Some(100)
        );
        assert_eq!(score_search_query("hello world", text, &tokens), Some(50));
        assert_eq!(score_search_query("tele arch", text, &tokens), Some(20));
        assert_eq!(score_search_query("nonexistent", text, &tokens), None);
    }

    #[test]
    fn ranking_tie_breaking_is_deterministic() {
        let ord1 = compare_search_results(100, 2000, 1, 10, 100, 1000, 1, 10);
        assert_eq!(ord1, Ordering::Less);

        let ord2 = compare_search_results(100, 1000, 1, 10, 100, 1000, 2, 5);
        assert_eq!(ord2, Ordering::Less);
    }

    #[test]
    fn ranking_streams_bounded_top_results_across_shards() {
        let mut collector = BoundedTopResults::new(50);

        for shard_id in 1..=120 {
            for item_idx in 1..=10 {
                let score = if (shard_id == 1 || shard_id == 60 || shard_id == 120) && item_idx == 1
                {
                    100
                } else {
                    10 + (shard_id % 20) as u32
                };

                let res = BoundedSearchResult {
                    score,
                    date: 1700000000 + (shard_id * 100 + item_idx) as i64,
                    peer_id: 100,
                    msg_id: (shard_id * 100 + item_idx) as i64,
                    entry_id: format!("shard_{shard_id}_item_{item_idx}"),
                };

                collector.insert(res);
            }
        }

        let top = collector.results();
        assert_eq!(top.len(), 50);

        let entry_ids: Vec<String> = top.iter().map(|r| r.entry_id.clone()).collect();
        assert!(entry_ids.contains(&"shard_1_item_1".to_string()));
        assert!(entry_ids.contains(&"shard_60_item_1".to_string()));
        assert!(entry_ids.contains(&"shard_120_item_1".to_string()));

        assert_eq!(top[0].score, 100);
        assert_eq!(top[1].score, 100);
        assert_eq!(top[2].score, 100);
        assert_eq!(top[0].entry_id, "shard_120_item_1");
        assert_eq!(top[1].entry_id, "shard_60_item_1");
        assert_eq!(top[2].entry_id, "shard_1_item_1");
    }

    #[test]
    fn ranking_multi_shard_equal_scores_order_deterministically() {
        let mut collector = BoundedTopResults::new(10);

        collector.insert(BoundedSearchResult {
            score: 50,
            date: 1000,
            peer_id: 200,
            msg_id: 10,
            entry_id: "p200_d1000".to_string(),
        });
        collector.insert(BoundedSearchResult {
            score: 50,
            date: 2000,
            peer_id: 100,
            msg_id: 5,
            entry_id: "p100_d2000".to_string(),
        });
        collector.insert(BoundedSearchResult {
            score: 50,
            date: 1000,
            peer_id: 100,
            msg_id: 20,
            entry_id: "p100_d1000_m20".to_string(),
        });
        collector.insert(BoundedSearchResult {
            score: 50,
            date: 1000,
            peer_id: 100,
            msg_id: 10,
            entry_id: "p100_d1000_m10".to_string(),
        });

        let res = collector.results();
        assert_eq!(res[0].entry_id, "p100_d2000");
        assert_eq!(res[1].entry_id, "p100_d1000_m10");
        assert_eq!(res[2].entry_id, "p100_d1000_m20");
        assert_eq!(res[3].entry_id, "p200_d1000");
    }
}
