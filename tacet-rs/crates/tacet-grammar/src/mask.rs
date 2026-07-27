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
//! MEASUREMENT (32k tokens, release, M-series; per step):
//!   structural positions (object start / key / enum) ..... 2-3 µs
//!   free string body .................................... ~0.95 ms
//! Free text is expensive because almost the WHOLE vocabulary is valid there
//! (~31k tokens), that is, there is no branch to prune. Since the per-step
//! budget of a 3B model is ~33ms, even this worst case is about 3% of the budget.

use crate::{AllowedSet, GrammarState};

/// A node of the prefix tree.
#[derive(Debug, Default)]
struct TrieNode {
    /// (character, child index) — kept sorted, so it can be binary-searched.
    children: Vec<(char, usize)>,
    /// The ids of the tokens that end at this node. `Vec`: there are
    /// vocabularies with two tokens that have the same text; both must be masked.
    ends: Vec<usize>,
}

/// The mask producer, built once over the vocabulary and reused again and again.
#[derive(Debug)]
pub struct TokenMask {
    nodes: Vec<TrieNode>,
    vocab_size: usize,
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
                current = match nodes[current].children.binary_search_by_key(&c, |(k, _)| *k) {
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
        Self { nodes, vocab_size: vocab.len(), empty_tokens }
    }

    /// The ids of tokens with empty (special) text; the caller decides on EOS
    /// with these.
    pub fn empty_tokens(&self) -> &[usize] {
        &self.empty_tokens
    }

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
        let mut mask = vec![false; self.vocab_size];
        let allowed = state.allowed_prefixes();
        self.walk(0, state, &allowed, terminator, &mut mask);
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
        let Ok(i) = self.nodes[node].children.binary_search_by_key(&term, |(k, _)| *k) else {
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
    fn walk(
        &self,
        node: usize,
        state: &GrammarState,
        allowed: &AllowedSet,
        terminator: Option<char>,
        mask: &mut [bool],
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
                for id in &self.nodes[*child].ends {
                    mask[*id] = true;
                }
                self.walk(*child, state, allowed, terminator, mask);
                continue;
            }
            let Ok(next) = state.branch(*c) else { continue };
            for id in &self.nodes[*child].ends {
                mask[*id] = true;
            }
            let next_allowed = next.allowed_prefixes();
            self.walk(*child, &next, &next_allowed, terminator, mask);
        }
    }
}
