//! `TokenMask` — tells which tokens of the vocabulary can be produced right now.
//!
//! HOT PATH: this function runs once for EVERY token produced. The naive method
//! (clone the state for each token and feed the text from scratch) processes
//! ~150k characters per step on a 32k vocabulary and recomputes the common
//! prefixes of tokens over and over.
//!
//! So the vocabulary is put into a prefix tree (trie) ONCE and the mask is
//! produced by walking the tree depth-first: the moment a prefix is rejected by
//! the grammar, ALL tokens BELOW that branch are eliminated in one move. In
//! practice the number of valid prefixes is very small (a few hundred branches
//! at the current JSON position), so the work scales with the number of valid
//! branches, not with the size of the vocabulary.
//!
//! NO tokenizer dependency: the interface is `&[String]`. Whichever tokenizer is
//! used, this crate does not change.
//!
//! MEASUREMENT, REDONE ON A REAL 151k VOCABULARY (qwen3-4b, release, M-series,
//! per step — `what_one_mask_step_costs_on_a_real_vocabulary` in tacet-cli):
//!
//! ```text
//!                                 before      after     open tokens
//! call start .................    0.072 ms    0.065 ms      439
//! inside a key ...............    0.028 ms    0.025 ms        4
//! free string body ...........    7.195 ms    0.137 ms  147 244
//! ```
//!
//! THE OLD NUMBERS HERE WERE 32k AND THEY DID NOT SCALE. This header used to
//! read "free string body ~0.95 ms" and conclude that even the worst case was
//! "about 3% of the budget" of a 3B model. The cost is close to linear in the
//! vocabulary, so on qwen3's 151k tokens it was 7.2 ms — and the conclusion, an
//! extrapolation nobody had rerun, was wrong by the same factor. It was found by
//! watching a selection case spend 35 s on a single generation.
//!
//! WHAT THE FAST PATH IS. Inside an UNBOUNDED string body the walk was not
//! deciding anything: `is_neutral` accepts every character except `"`, `\` and
//! the controls, so the answer is "every token carrying none of those" — a
//! property of the vocabulary, not of the state. It is precomputed once
//! (`plain`) and each subtree that holds no break character is skipped, which on
//! this vocabulary is 97% of the trie. Same 147 244 tokens open, 52x less time.
//!
//! THE ANSWER IS UNCHANGED AND THAT IS TESTED, not argued: the property suite's
//! `the_mask_and_the_automaton_never_disagree` compares the mask against
//! `advance` in both directions over generated schemas, and
//! `the_free_text_fast_path_agrees_with_the_automaton_token_by_token` does it on
//! a vocabulary built to put break characters at every position.

use crate::{AllowedSet, GrammarState};

/// A node of the prefix tree.
#[derive(Debug, Default)]
struct TrieNode {
    /// (character, child index) — kept sorted, so it can be binary-searched.
    children: Vec<(char, usize)>,
    /// The ids of the tokens that end at this node. `Vec`: there are
    /// vocabularies with two tokens that have the same text; both must be masked.
    ends: Vec<usize>,
    /// Does ANY path below this node (including the edge into it) carry a
    /// character that ends a free-text run — `\"`, `\\` or a control.
    ///
    /// This is what lets the free-text walk stop early: a subtree with no such
    /// character contains only tokens already opened by `plain`, so descending
    /// it would re-mark bits that are set. On this vocabulary that is 97% of the
    /// trie.
    subtree_breaks: bool,
}

/// The mask producer, built once over the vocabulary and reused again and again.
#[derive(Debug)]
pub struct TokenMask {
    nodes: Vec<TrieNode>,
    vocab_size: usize,
    /// The tokens whose text carries no `\"`, no `\\` and no control character.
    ///
    /// IN AN UNBOUNDED STRING BODY THIS IS THE ANSWER, already computed. Every
    /// one of those tokens is reachable by a run of neutral characters, and
    /// `GrammarState::is_neutral` accepts exactly the characters that are not in
    /// `breaks_free_text` — so the walk would mark precisely this set and nothing
    /// else, one trie node at a time. Built once, cloned per step.
    plain: Vec<bool>,
    /// Tokens with empty text (special/control tokens). As far as the grammar is
    /// concerned they are neutral; they are always left closed in the mask —
    /// when special tokens such as EOS become free is the caller's decision
    /// (`GrammarState::is_done`).
    empty_tokens: Vec<usize>,
}

impl TokenMask {
    /// Turns the vocabulary into a prefix tree. The cost is paid once.
    pub fn new(vocab: &[String]) -> Self {
        let mut nodes = vec![TrieNode::default()];
        let mut empty_tokens = Vec::new();
        for (id, text) in vocab.iter().enumerate() {
            if text.is_empty() {
                empty_tokens.push(id);
                continue;
            }
            let mut current = 0usize;
            for c in text.chars() {
                current = match nodes[current]
                    .children
                    .binary_search_by_key(&c, |(k, _)| *k)
                {
                    Ok(i) => nodes[current].children[i].1,
                    Err(i) => {
                        nodes.push(TrieNode::default());
                        let fresh = nodes.len() - 1;
                        nodes[current].children.insert(i, (c, fresh));
                        fresh
                    }
                };
            }
            nodes[current].ends.push(id);
        }

        // THE TWO PRECOMPUTATIONS THE FREE-TEXT FAST PATH RESTS ON.
        //
        // `plain` is the answer for an unbounded string body, and it is a
        // property of the VOCABULARY alone — no state can change which tokens
        // carry a quote. An empty token stays closed here for the same reason it
        // is closed everywhere else: whether EOS is free is `is_done`'s call.
        let plain: Vec<bool> = vocab
            .iter()
            .map(|t| !t.is_empty() && !t.chars().any(crate::state::breaks_free_text))
            .collect();

        // `subtree_breaks` is filled in REVERSE INDEX ORDER, and that is a
        // post-order traversal for free: a child node is always pushed after its
        // parent above, so every child index is greater than its parent's. A
        // recursive pass would risk the stack on a deep vocabulary; this cannot.
        for i in (0..nodes.len()).rev() {
            let breaks = nodes[i].children.iter().any(|(c, child)| {
                crate::state::breaks_free_text(*c) || nodes[*child].subtree_breaks
            });
            nodes[i].subtree_breaks = breaks;
        }

        Self {
            nodes,
            vocab_size: vocab.len(),
            plain,
            empty_tokens,
        }
    }

    /// The ids of tokens with empty (special) text; the caller decides on EOS
    /// with these.
    pub fn empty_tokens(&self) -> &[usize] {
        &self.empty_tokens
    }

    /// How many token ids this mask knows about. A logit slice longer than this
    /// has ids the mask can say nothing about, and the caller closes those —
    /// silence is not permission.
    pub fn vocab_size(&self) -> usize {
        self.vocab_size
    }

    /// `true` for the tokens that can be produced in the given grammar state.
    pub fn mask(&self, state: &GrammarState) -> Vec<bool> {
        self.mask_with_terminator(state, None)
    }

    /// A mask that, on top of the grammar, also recognizes a TERMINATOR character.
    ///
    /// WHY IT IS NEEDED (found by measurement): the grammar only describes the
    /// ARGUMENTS; the `)` that closes the call is OUTSIDE it. But the tokenizer
    /// does not know that boundary. In the Qwen2.5 vocabulary the NATURAL
    /// tokenization of the string `calculate({"expression": "12*8"})` ends with `"})`
    /// (id 80154) — that is, the token the model is MOST LIKELY to produce sits
    /// exactly on the grammar boundary.
    ///
    /// Had we walked only inside the grammar without recognizing the terminator,
    /// that token would stay CLOSED in the mask; the model could not write a
    /// valid call in one move, it would have to split it into `"}` + `)`. Worse,
    /// `advance` DID accept that same token: masking and advancing drift apart,
    /// meaning the constraint would make its own accepted output impossible to
    /// produce.
    ///
    /// The fix is a natural part of the walk: while the grammar is in an
    /// ACCEPTING state the terminator edge is walked too, and the tokens that
    /// END there are opened. We do not descend PAST the terminator — the call
    /// closes there, what follows is no longer the grammar's business and would
    /// allow chatter to be appended after the call.
    pub fn mask_with_terminator(
        &self,
        state: &GrammarState,
        terminator: Option<char>,
    ) -> Vec<bool> {
        // THE FAST PATH, and it is the difference between 7.2 ms and a memcpy on
        // a 151k vocabulary — measured, see the module header.
        //
        // In an unbounded string body every token made only of neutral
        // characters is producible, so the walk's job there is not to DECIDE
        // anything, it is to re-derive `plain` one node at a time. Starting from
        // `plain` and telling the walk it is covered leaves the walk exactly the
        // work that is still a decision: the paths through `\"`, `\\` and the
        // control characters.
        let covered = state.in_free_text_run();
        let mut mask = if covered {
            self.plain.clone()
        } else {
            vec![false; self.vocab_size]
        };
        let allowed = state.allowed_prefixes();
        self.walk(0, state, &allowed, terminator, &mut mask, covered);
        mask
    }

    /// If the grammar can close right now, opens the tokens that END on the
    /// terminator edge leaving this node. A separate function: it is called from
    /// two places inside `walk` (the root and every advancing branch) and the
    /// condition should live in one place.
    fn open_terminator(
        &self,
        node: usize,
        state: &GrammarState,
        terminator: Option<char>,
        mask: &mut [bool],
    ) {
        let Some(term) = terminator else { return };
        if !state.is_done() {
            return;
        }
        let Ok(i) = self.nodes[node]
            .children
            .binary_search_by_key(&term, |(k, _)| *k)
        else {
            return;
        };
        let child = self.nodes[node].children[i].1;
        for id in &self.nodes[child].ends {
            mask[*id] = true;
        }
    }

    /// `allowed` is always the allowed set of `state`; it is passed as a
    /// parameter because on neutral branches the state does not change, so
    /// neither does the set — recomputing it would mean one allocation per node.
    /// `plain_covered`: every character from the ROOT to this node was neutral,
    /// so `mask` already holds `plain` for everything below. It is handed down
    /// the neutral branch and reset to `false` the moment the walk takes an
    /// advancing edge — a token like `\\nabc` carries no break character AFTER
    /// the escape, but its path from the root does, so it is not in `plain` and
    /// its subtree still has to be walked.
    fn walk(
        &self,
        node: usize,
        state: &GrammarState,
        allowed: &AllowedSet,
        terminator: Option<char>,
        mask: &mut [bool],
        plain_covered: bool,
    ) {
        // If the grammar can close at this node, the token that ends the call is
        // legitimate too.
        self.open_terminator(node, state, terminator, mask);
        for (c, child) in &self.nodes[node].children {
            // The cheap check first: is the character valid at this position. If
            // it is, advance the state — the clone is only paid on valid branches.
            if !allowed.contains(*c) {
                continue;
            }
            // A neutral character does not advance the automaton: descend with
            // the same state without paying for a clone at all. In free-text
            // areas almost the entire walk takes this branch. Otherwise really
            // advance the state — a token is opened in the mask only AFTER the
            // transition is VERIFIED; `allowed` is the fast pre-filter, the
            // automaton has the final word.
            if state.is_neutral(*c) {
                // Nothing below carries a break character, and the path here was
                // all neutral: every token down there is in `plain`, which the
                // mask already holds. Descending would set bits that are set.
                if plain_covered && !self.nodes[*child].subtree_breaks {
                    continue;
                }
                for id in &self.nodes[*child].ends {
                    mask[*id] = true;
                }
                self.walk(*child, state, allowed, terminator, mask, plain_covered);
                continue;
            }
            let Ok(next) = state.branch(*c) else { continue };
            for id in &self.nodes[*child].ends {
                mask[*id] = true;
            }
            let next_allowed = next.allowed_prefixes();
            self.walk(*child, &next, &next_allowed, terminator, mask, false);
        }
    }
}
