use ahash::{AHashMap, AHashSet};
use ebnf::regex::FiniteStateAutomaton;
use fixedbitset::FixedBitSet;
use jaggedarray::jagged_array::JaggedArray;
use jaggedarray::jagged_array::JaggedArrayViewTrait;
use num::{
    cast::AsPrimitive,
    traits::{ConstOne, ConstZero, NumAssign, NumOps},
    Num,
};
use regex_automata::dfa::Automaton;
use regex_automata::hybrid::dfa::Cache;
use regex_automata::hybrid::LazyStateID;
use regex_automata::util::primitives::StateID;
use std::borrow::BorrowMut;
use std::sync::Arc;

use crate::grammar::ExceptedID;
use crate::grammar::RegexID;
use crate::utils::ByteSet;
use crate::{
    grammar::{Grammar, LNFNode, NonterminalID},
    vocabulary::Vocabulary,
};
type EarleySets<TN, TD, TP, TSP, TS> = JaggedArray<EarleyItem<TN, TD, TP, TSP, TS>, Vec<usize>, 2>;
const USIZE_WIDTH: usize = std::mem::size_of::<usize>();
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct EarleyItem<TN, TD, TP, TSP, TS>
where
    TN: Num + AsPrimitive<usize> + ConstOne + ConstZero,
    TD: Num + AsPrimitive<usize> + ConstOne + ConstZero,
    TP: Num + AsPrimitive<usize> + ConstOne + ConstZero,
    TSP: Num + AsPrimitive<usize> + ConstOne + ConstZero,
    usize: num::traits::AsPrimitive<TN>
        + num::traits::AsPrimitive<TD>
        + num::traits::AsPrimitive<TP>
        + num::traits::AsPrimitive<TSP>,
{
    pub nonterminal_id: NonterminalID<TN>,
    pub dot_position: TD,
    pub production_index: TP,
    pub start_position: TSP,
    pub state_id: TS,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ToBeCompletedItem<TN, TSP>
where
    TN: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TSP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
{
    nonterminal_id: NonterminalID<TN>,
    start_position: TSP,
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub cache_enabled: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("Terminal length {0} exceeds {1}, the maximum terminal length allowed by current size of StateID(TS).
     Consider reducing terminal length or use larger StateID(TS).")]
    TerminalTooLong(usize, usize),
    #[error("Regex length {0} exceeds {1}, the maximum regex length allowed by current size of StateID(TS).
     Consider reducing regex states or use larger StateID(TS).")]
    RegexTooLarge(usize, usize),
    #[error("Except! length {0} exceeds {1}, the maximum excepted length allowed by current size of StateID(TS).
     Consider reducing excepted terminals, use larger StateID(TS) or less repetition.")]
    ExceptedTooLarge(usize, usize),
    #[error("Repetition in regex {0} exceeds {1}, the maximum repetition allowed by current size of StateID(TS).
     Consider reducing repetition or use larger StateID(TS).")]
    RepetitionInExceptedTooLarge(usize, usize),
}

#[derive(Debug, Clone)]
pub struct Engine<TI, TE, TD, TP, TSP, TS>
where
    TI: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TE: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TD: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TSP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TS: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    usize: num::traits::AsPrimitive<TI>
        + num::traits::AsPrimitive<TD>
        + num::traits::AsPrimitive<TP>
        + num::traits::AsPrimitive<TSP>,
{
    vocabulary: Arc<Vocabulary>,
    grammar: Arc<Grammar<TI, TE>>,
    allowed_first_bytes: ByteSet,
    allowed_token_ids: FixedBitSet,
    earley_sets: EarleySets<TI, TD, TP, TSP, TS>,
    cache: AHashMap<EarleySets<TI, TD, TP, TSP, TS>, FixedBitSet>,
    regex_id_to_cache: AHashMap<RegexID<TI>, Cache>,
    excepted_id_to_cache: AHashMap<ExceptedID<TI>, Cache>,
    to_be_completed_items: AHashSet<ToBeCompletedItem<TI, TSP>>,
    already_predicted_nonterminals: FixedBitSet,
    config: EngineConfig,
    regex_start_config: regex_automata::util::start::Config,
    excepted_start_config: regex_automata::util::start::Config,
}

impl<TI, TE, TD, TP, TSP, TS> Engine<TI, TE, TD, TP, TSP, TS>
where
    TI: Num
        + AsPrimitive<usize>
        + ConstOne
        + ConstZero
        + NumOps
        + NumAssign
        + std::cmp::PartialOrd
        + num::Bounded
        + std::convert::From<usize>,
    TI: Eq + std::hash::Hash + PartialEq,
    TE: Num
        + AsPrimitive<usize>
        + ConstOne
        + ConstZero
        + Eq
        + std::hash::Hash
        + PartialEq
        + num::Bounded
        + std::convert::From<usize>,
    TD: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TSP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TS: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    usize: num::traits::AsPrimitive<TI>
        + num::traits::AsPrimitive<TE>
        + num::traits::AsPrimitive<TD>
        + num::traits::AsPrimitive<TP>
        + num::traits::AsPrimitive<TSP>
        + num::traits::AsPrimitive<TS>,
{
    const STATE_ID_TYPE_SIZE: usize = std::mem::size_of::<TS>();
    pub fn new(
        vocabulary: Arc<Vocabulary>,
        grammar: Arc<Grammar<TI, TE>>,
        config: EngineConfig,
    ) -> Result<Self, EngineError> {
        // Verify necessary conditions
        assert!(
            Self::STATE_ID_TYPE_SIZE <= USIZE_WIDTH,
            "state id type size {} is larger than usize width: {}",
            Self::STATE_ID_TYPE_SIZE,
            USIZE_WIDTH
        );
        Self::validate_ts_size_for_terminals(&grammar)?;
        Self::validate_ts_size_for_regexes(&grammar)?;
        Self::validate_ts_size_for_excepted(&grammar)?;
        // Init fields
        let allowed_first_bytes = ByteSet::with_capacity(u8::MAX as usize);
        let allowed_token_ids = FixedBitSet::with_capacity(vocabulary.get_vocab_size());
        let mut earley_sets = JaggedArray::new();
        earley_sets.new_row::<0>();
        let cache = AHashMap::default();
        let to_be_completed_items = AHashSet::default();
        let already_predicted_nonterminals =
            FixedBitSet::with_capacity(grammar.get_nonterminals_size());
        let regex_id_to_cache = AHashMap::default();
        let excepted_id_to_cache = AHashMap::default();
        let start = grammar.get_start_nonterminal_id();
        let mut engine = Self {
            vocabulary,
            grammar,
            allowed_first_bytes,
            allowed_token_ids,
            earley_sets,
            cache,
            to_be_completed_items,
            already_predicted_nonterminals,
            config,
            regex_start_config: regex_automata::util::start::Config::new()
                .anchored(regex_automata::Anchored::Yes),
            excepted_start_config: regex_automata::util::start::Config::new()
                .anchored(regex_automata::Anchored::No),
            regex_id_to_cache,
            excepted_id_to_cache,
        };
        engine.predict_nonterminal(start, 0); // init the first earley set
        engine.predict(); // run a full prediction for the first earley set
        engine.update_allowed_first_bytes();
        Ok(engine)
    }

    fn validate_ts_size_for_terminals(grammar: &Grammar<TI, TE>) -> Result<(), EngineError> {
        let terminals = grammar.get_id_to_terminals();
        let max: usize = (1 << (Self::STATE_ID_TYPE_SIZE * 8)) - 1;
        for i in 0..terminals.len() {
            let terminal = terminals.view::<1, 1>([i]);
            if terminal.len() > max {
                return Err(EngineError::TerminalTooLong(terminal.len(), max));
            }
        }
        Ok(())
    }

    fn validate_ts_size_for_regexes(grammar: &Grammar<TI, TE>) -> Result<(), EngineError> {
        let regexes = grammar.get_id_to_regexes();
        let max: usize = (1 << (Self::STATE_ID_TYPE_SIZE * 8)) - 1;
        for fsa in regexes {
            match fsa {
                FiniteStateAutomaton::Dfa(dfa) => {
                    if dfa.state_len() > max {
                        return Err(EngineError::RegexTooLarge(dfa.state_len(), max));
                    }
                }
                FiniteStateAutomaton::LazyDFA(_) => {
                    if LazyStateID::MAX > max {
                        return Err(EngineError::RegexTooLarge(LazyStateID::MAX, max));
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_ts_size_for_excepted(grammar: &Grammar<TI, TE>) -> Result<(), EngineError> {
        let rules = grammar.get_rules();
        for i in 0..rules.len() {
            let productions = rules.view::<1, 2>([i]);
            for j in 0..productions.len() {
                let column = productions.view::<1, 1>([j]);
                for k in 0..column.len() {
                    let node = column[[k]];
                    if let LNFNode::EXCEPT(id, _) = node {
                        // repetition is verified in grammar
                        let fsa = grammar.get_excepted(id);
                        let max: usize =
                            (1 << ((Self::STATE_ID_TYPE_SIZE - std::mem::size_of::<TE>()) * 8)) - 1;
                        match fsa {
                            FiniteStateAutomaton::Dfa(dfa) => {
                                if dfa.state_len() > max {
                                    return Err(EngineError::ExceptedTooLarge(
                                        dfa.state_len(),
                                        max,
                                    ));
                                }
                            }
                            FiniteStateAutomaton::LazyDFA(_) => {
                                if LazyStateID::MAX > max {
                                    return Err(EngineError::ExceptedTooLarge(
                                        LazyStateID::MAX,
                                        max,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Run prediction stage of Earley algorithm.
    fn predict(&mut self) {
        let earley_set_index = self.earley_sets.len() - 1;
        let mut earley_set_len = self.earley_sets.view::<1, 1>([earley_set_index]).len();
        let mut i = 0;
        while i < earley_set_len {
            let item = self.earley_sets[[earley_set_index, i]];
            let node = *self.grammar.get_node(
                item.nonterminal_id,
                item.dot_position,
                item.production_index,
            );
            if let LNFNode::Nonterminal(nonterminal_id) = node {
                earley_set_len += self.predict_nonterminal(nonterminal_id, earley_set_index);
            }
            i += 1;
        }
    }
    /// Predict one nonterminal according to Earley algorithm.
    /// This function ensures no duplication happens.
    /// Returns earley set length increment due to prediction
    fn predict_nonterminal(
        &mut self,
        nonterminal_id: NonterminalID<TI>,
        earley_set_index: usize,
    ) -> usize {
        let nid = nonterminal_id.0.as_();
        if !self.already_predicted_nonterminals.contains(nid) {
            self.already_predicted_nonterminals.insert(nid);
            let production_len = self.grammar.get_production_len(nonterminal_id);
            for j in 0..production_len {
                let production_index = j.as_();
                let new_item = EarleyItem {
                    nonterminal_id,
                    dot_position: TD::ZERO,
                    production_index,
                    start_position: earley_set_index.as_(),
                    state_id: match self.grammar.get_node(
                        nonterminal_id,
                        TD::ZERO,
                        production_index,
                    ) {
                        &LNFNode::RegexString(id) => {
                            let fsa = self.grammar.get_regex(id);
                            match fsa {
                                FiniteStateAutomaton::Dfa(dfa) => {
                                    // SAFETY: start_error will not happen since that will result in an error in Grammar::new() method
                                    let start = dfa.start_state(&self.regex_start_config).unwrap();
                                    Self::from_dfa_state_id_to_state_id(start, dfa.stride2())
                                }
                                FiniteStateAutomaton::LazyDFA(dfa) => {
                                    // SAFETY: start_error will not happen since that will result in an error in Grammar::new() method
                                    let start = dfa
                                        .start_state(
                                            self.regex_id_to_cache.get_mut(&id).unwrap(),
                                            &self.regex_start_config,
                                        )
                                        .unwrap();
                                    Self::from_ldfa_state_id_to_state_id(start)
                                }
                            }
                        }
                        LNFNode::EXCEPT(id, r) => {
                            let fsa = self.grammar.get_excepted(*id);
                            match fsa {
                                FiniteStateAutomaton::Dfa(dfa) => {
                                    // SAFETY: start_error will not happen since that will result in an error in Grammar::new() method
                                    let start =
                                        dfa.start_state(&self.excepted_start_config).unwrap();
                                    match r {
                                        Some(r) => Self::from_dfa_state_id_to_state_id_with_r(
                                            start,
                                            dfa.stride2(),
                                            *r,
                                        ),
                                        None => Self::from_dfa_state_id_to_state_id(
                                            start,
                                            dfa.stride2(),
                                        ),
                                    }
                                }
                                FiniteStateAutomaton::LazyDFA(dfa) => {
                                    // SAFETY: start_error will not happen since that will result in an error in Grammar::new() method
                                    let start = dfa
                                        .start_state(
                                            self.excepted_id_to_cache.get_mut(&id).unwrap(),
                                            &self.excepted_start_config,
                                        )
                                        .unwrap();
                                    match r {
                                        Some(r) => {
                                            Self::from_ldfa_state_id_to_state_id_with_r(start, *r)
                                        }
                                        None => Self::from_ldfa_state_id_to_state_id(start),
                                    }
                                }
                            }
                        }
                        _ => TS::ZERO,
                    },
                };
                self.earley_sets.push_to_last_row(new_item);
            }
            production_len
        } else {
            0
        }
    }
    /// This function requires the last Earley set has been created and fully predicted.
    fn update_allowed_first_bytes(&mut self) {
        self.allowed_first_bytes.clear();
        let earley_set_index = self.earley_sets.len() - 1;
        let earley_set = self.earley_sets.view::<1, 1>([earley_set_index]).as_slice();
        for item in earley_set.iter() {
            let node = *self.grammar.get_node(
                item.nonterminal_id,
                item.dot_position,
                item.production_index,
            );
            match node {
                LNFNode::Terminal(terminal_id) => {
                    self.allowed_first_bytes
                        .insert(self.grammar.get_terminal(terminal_id)[0].as_());
                }
                LNFNode::RegexString(regex_id) => {
                    self.allowed_first_bytes
                        .union_with(self.grammar.get_first_bytes_from_regex(regex_id));
                }
                LNFNode::EXCEPT(excepted_id, _) => {
                    self.allowed_first_bytes
                        .union_with(self.grammar.get_first_bytes_from_excepted(excepted_id));
                }
                _ => {}
            }
        }
    }
    #[inline]
    fn item_should_be_completed(
        &self,
        nonterminal_id: NonterminalID<TI>,
        new_dot_position: TD,
        production_id: TP,
    ) -> bool
    where
        TP: Num + AsPrimitive<usize> + ConstOne + ConstZero,
        TD: Num + AsPrimitive<usize> + ConstOne + ConstZero,
    {
        let view = self.grammar.get_dotted_productions(nonterminal_id);
        if new_dot_position.as_() < view.len() {
            let view = view.view::<1, 1>([new_dot_position.as_()]);
            if production_id.as_() < view.len() {
                return false;
            }
        }
        return true;
    }

    /// This function requires the newest Earley set has been created.
    fn advance_item(&mut self, item: EarleyItem<TI, TD, TP, TSP, TS>) {
        let new_dotted_position = item.dot_position + 1.as_();
        if self.item_should_be_completed(
            item.nonterminal_id,
            new_dotted_position,
            item.production_index,
        ) {
            self.to_be_completed_items.insert(ToBeCompletedItem {
                nonterminal_id: item.nonterminal_id,
                start_position: item.start_position,
            });
        } else {
            let new_item = EarleyItem {
                nonterminal_id: item.nonterminal_id,
                dot_position: new_dotted_position,
                production_index: item.production_index,
                start_position: item.start_position,
                state_id: item.state_id,
            };
            self.earley_sets.push_to_last_row(new_item);
        }
    }
    /// ### SAFETY
    ///
    /// This function can only be called when state_id is actually a index(integers within usize range), otherwise it will produce invalid index.
    #[inline]
    unsafe fn from_state_id_to_index(state_id: TS) -> usize {
        state_id.as_()
    }
    #[inline]
    fn from_dfa_state_id_to_state_id(state_id: StateID, stride2: usize) -> TS {
        // SAFETY: state_id is a u32 due to #[repr(transparent)] attribute
        let id: u32 = unsafe { std::mem::transmute(state_id) };
        // SAFETY: id is guaranteed to be representable as a state_id or an error will be returned in Self::new() method
        ((id >> stride2) as usize).as_()
    }
    #[inline]
    fn from_dfa_state_id_to_state_id_with_r(state_id: StateID, stride2: usize, r: TE) -> TS {
        // SAFETY: state_id is a u32 due to #[repr(transparent)] attribute
        let id: u32 = unsafe { std::mem::transmute(state_id) };
        // SAFETY: id is guaranteed to be representable as a state_id or an error will be returned in Self::new() method
        let a = ((id >> stride2) as usize)
            + (r.as_() << ((std::mem::size_of::<TS>() - std::mem::size_of::<TE>()) * 8));
        a.as_()
    }
    #[inline]
    fn from_ldfa_state_id_to_state_id(state_id: LazyStateID) -> TS {
        // SAFETY: state_id is a u32 due to #[repr(transparent)] attribute
        let id: u32 = unsafe { std::mem::transmute(state_id) };
        // SAFETY: id is guaranteed to be representable as a state_id or an error will be returned in Self::new() method
        (id as usize).as_()
    }
    #[inline]
    fn from_ldfa_state_id_to_state_id_with_r(state_id: LazyStateID, r: TE) -> TS {
        // SAFETY: state_id is a u32 due to #[repr(transparent)] attribute
        let id: u32 = unsafe { std::mem::transmute(state_id) };
        // SAFETY: id is guaranteed to be representable as a state_id or an error will be returned in Self::new() method
        let a = (id as usize)
            + (r.as_() << ((std::mem::size_of::<TS>() - std::mem::size_of::<TE>()) * 8));
        a.as_()
    }
    /// ### SAFETY
    ///
    /// This function can only be called when state_id is actually a index AND index can be represented as a state_id
    #[inline]
    unsafe fn from_index_to_state_id(index: usize) -> TS {
        index.as_()
    }

    fn scan(&mut self, byte: u8) {
        let earley_set_index = self.earley_sets.len() - 1;
        let earley_set_len = self.earley_sets.view::<1, 1>([earley_set_index]).len();
        for i in 0..earley_set_len {
            let item = self.earley_sets[[earley_set_index, i]];
            let node = *self.grammar.get_node(
                item.nonterminal_id,
                item.dot_position,
                item.production_index,
            );
            match node {
                LNFNode::Terminal(terminal_id) => {
                    let terminal = self.grammar.get_terminal(terminal_id);
                    // SAFETY: state_id is guaranteed to be a valid index due to the terminal branch
                    let index = unsafe { Self::from_state_id_to_index(item.state_id) };
                    if terminal[index] == byte {
                        let index = index + 1;
                        if index < terminal.len() {
                            // SAFETY: state_id is guaranteed to be a valid index and the check in Self::new() ensures it is representable as a state id
                            let new_state_index = unsafe { Self::from_index_to_state_id(index) };
                            let new_item = EarleyItem {
                                nonterminal_id: item.nonterminal_id,
                                dot_position: item.dot_position,
                                production_index: item.production_index,
                                start_position: item.start_position,
                                state_id: new_state_index,
                            };
                            self.earley_sets.push_to_last_row(new_item);
                        } else {
                            self.advance_item(item);
                        }
                    }
                }
                LNFNode::RegexString(regex_id) => {
                    let regex = self.grammar.get_regex(regex_id);
                }
            }
        }
    }

    fn advance_fsa(fsa: &FiniteStateAutomaton, state_id: TS, byte: u8) -> Vec<usize> {
        let mut result = vec![];
        let mut state = fsa.get_state(state_id);
        for transition in state.transitions.iter() {
            if transition.0.contains(byte) {
                result.push(transition.1);
            }
        }
        result
    }
}
