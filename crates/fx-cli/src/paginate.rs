//! Paging over GitFox list endpoints.
//!
//! GitFox declares no pagination headers — no `X-Total`, no `Link` — so the
//! only way to learn whether more results exist is to ask for one more than you
//! want and see whether it arrives. That is what [`collect`] does, which makes
//! `truncated` an observation rather than a guess.
//!
//! Why this matters: without it, every list command silently stops at its page
//! size. A caller asking for "the open pull requests" would be handed thirty of
//! them with nothing to say there were forty, and an agent would act on the
//! wrong picture.

use std::future::Future;

/// The largest page GitFox accepts (`limit` is capped at 100 across its list
/// endpoints).
pub const MAX_PAGE_SIZE: u32 = 100;

pub struct Paged<T> {
    pub items: Vec<T>,
    /// Whether the server had more beyond what was returned.
    pub truncated: bool,
}

/// Fetch pages until `want` items are in hand, or the server runs out.
///
/// `fetch` is called with a 1-based page number and a page size, and must map
/// those onto the endpoint's own `page` and `limit` parameters.
///
/// The page size is fixed for the whole sequence. It has to be: `page` and
/// `limit` are an *offset* pair, so page 3 of size 6 is items 12 to 18 — not
/// "six more after the two hundred already seen". Shrinking the last page to
/// ask for exactly what is still missing silently reads from the wrong offset.
pub async fn collect<T, E, F, Fut>(want: u32, mut fetch: F) -> Result<Paged<T>, E>
where
    F: FnMut(u32, u32) -> Fut,
    Fut: Future<Output = Result<Vec<T>, E>>,
{
    let want = want.max(1) as usize;
    // One more than we need, when that fits in a page: receiving it proves
    // there is more without costing a second request.
    let size = (want + 1).min(MAX_PAGE_SIZE as usize) as u32;

    let mut items: Vec<T> = Vec::new();
    let mut page = 1u32;

    loop {
        let batch = fetch(page, size).await?;
        let received = batch.len();
        items.extend(batch);

        if items.len() > want {
            items.truncate(want);
            return Ok(Paged {
                items,
                truncated: true,
            });
        }
        // A short page is the end of the collection.
        if received < size as usize {
            return Ok(Paged {
                items,
                truncated: false,
            });
        }
        page += 1;
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// A server holding `total` items, recording the pages it was asked for.
    struct Fake {
        total: usize,
        calls: RefCell<Vec<(u32, u32)>>,
    }

    impl Fake {
        fn new(total: usize) -> Self {
            Self {
                total,
                calls: RefCell::new(Vec::new()),
            }
        }

        fn page(&self, page: u32, size: u32) -> Result<Vec<usize>, ()> {
            self.calls.borrow_mut().push((page, size));
            let start = (page as usize - 1) * size as usize;
            let end = (start + size as usize).min(self.total);
            Ok((start..end).collect())
        }

        fn calls(&self) -> Vec<(u32, u32)> {
            self.calls.borrow().clone()
        }
    }

    async fn run(total: usize, want: u32) -> (Paged<usize>, Vec<(u32, u32)>) {
        let fake = Fake::new(total);
        // Capture a reference, not the value: `async move` inside an `FnMut`
        // would otherwise move the server into the first future. Real call
        // sites hold the client the same way.
        let server = &fake;
        let paged = collect(
            want,
            move |page, size| async move { server.page(page, size) },
        )
        .await
        .unwrap();
        (paged, fake.calls())
    }

    #[tokio::test]
    async fn fewer_results_than_asked_for_is_not_truncated() {
        let (paged, calls) = run(12, 30).await;
        assert_eq!(paged.items.len(), 12);
        assert!(!paged.truncated);
        // One request: 31 asked for, 12 came back, so that is all of them.
        assert_eq!(calls, vec![(1, 31)]);
    }

    #[tokio::test]
    async fn exactly_as_many_as_asked_for_is_not_truncated_either() {
        let (paged, calls) = run(30, 30).await;
        assert_eq!(paged.items.len(), 30);
        assert!(
            !paged.truncated,
            "there were exactly 30; nothing was hidden"
        );
        assert_eq!(calls, vec![(1, 31)]);
    }

    #[tokio::test]
    async fn one_more_than_asked_for_is_reported_as_truncated() {
        let (paged, calls) = run(31, 30).await;
        assert_eq!(paged.items.len(), 30);
        assert!(paged.truncated);
        assert_eq!(calls, vec![(1, 31)], "still one request");
    }

    #[tokio::test]
    async fn a_large_request_is_paged_at_the_server_maximum() {
        let (paged, calls) = run(1000, 250).await;
        assert_eq!(paged.items.len(), 250);
        assert!(paged.truncated);
        // Every page the same size, because `page`/`limit` are an offset pair.
        assert_eq!(calls, vec![(1, 100), (2, 100), (3, 100)]);
        assert!(calls.iter().all(|(_, size)| *size <= MAX_PAGE_SIZE));
    }

    #[tokio::test]
    async fn paging_stops_early_when_the_collection_ends_mid_way() {
        let (paged, calls) = run(150, 500).await;
        assert_eq!(paged.items.len(), 150);
        assert!(!paged.truncated);
        assert_eq!(calls, vec![(1, 100), (2, 100)]);
    }

    /// The regression this module exists to prevent: with a shrinking last
    /// page, page 3 of size 6 reads items 12..18 rather than 200..206, and the
    /// caller is handed the wrong rows with no sign anything went wrong.
    #[tokio::test]
    async fn the_items_are_the_right_ones_in_the_right_order() {
        let (paged, _) = run(1000, 205).await;
        assert_eq!(paged.items.first(), Some(&0));
        assert_eq!(paged.items.last(), Some(&204));
        assert_eq!(paged.items.len(), 205);
    }

    #[tokio::test]
    async fn an_empty_collection_is_handled() {
        let (paged, calls) = run(0, 30).await;
        assert!(paged.items.is_empty());
        assert!(!paged.truncated);
        assert_eq!(calls.len(), 1);
    }

    #[tokio::test]
    async fn a_limit_of_zero_still_asks_for_something() {
        let (paged, calls) = run(10, 0).await;
        assert_eq!(paged.items.len(), 1, "clamped to one rather than looping");
        assert!(paged.truncated);
        assert_eq!(calls, vec![(1, 2)]);
    }

    #[tokio::test]
    async fn an_error_stops_paging_immediately() {
        let result: Result<Paged<usize>, &str> = collect(500, |page, _| async move {
            if page == 1 {
                Ok((0..100).collect())
            } else {
                Err("boom")
            }
        })
        .await;
        assert_eq!(result.err(), Some("boom"));
    }
}
