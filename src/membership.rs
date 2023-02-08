use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

use stateright::actor::{Actor, Id, Out};

use crate::stable_set::StableSet;
use crate::{
    fake_crypto::{SectionSig, Sig},
    stable_set::Member,
};

#[derive(
    Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub enum Msg {
    ReqJoin(Id, Member),
    JoinShare(u64, Id, Sig<(u64, Id)>, Member),
    Joined(u64, Id, SectionSig<(u64, Id)>),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct State {
    pub elders: BTreeSet<Id>,
    pub stable_set: StableSet,
    pub joining_section_sig: BTreeMap<u64, SectionSig<(u64, Id)>>,
}

#[derive(Clone)]
pub struct Node {
    pub genesis_nodes: BTreeSet<Id>,
    pub peers: Vec<Id>,
}

impl Actor for Node {
    type Msg = Msg;
    type State = State;

    fn on_start(&self, id: Id, o: &mut Out<Self>) -> Self::State {
        let elders = self.genesis_nodes.clone();
        let mut stable_set = StableSet::default();

        for node in self.genesis_nodes.iter().copied() {
            let mut sig = SectionSig::new(elders.clone());
            for genesis_signer in self.genesis_nodes.iter().copied() {
                sig.add_share(genesis_signer, Sig::sign(genesis_signer, (0, node)));
            }

            stable_set.add(0, node, sig);
        }

        if !self.genesis_nodes.contains(&id) {
            let last_member = stable_set.last_member().unwrap();
            o.broadcast(elders.iter(), &Msg::ReqJoin(id, last_member));
        }

        State {
            elders,
            stable_set,
            joining_section_sig: BTreeMap::new(),
        }
    }

    fn on_msg(
        &self,
        id: Id,
        state: &mut Cow<Self::State>,
        src: Id,
        msg: Self::Msg,
        o: &mut Out<Self>,
    ) {
        match msg {
            Msg::ReqJoin(candidate_id, member) => {
                if !state.stable_set.contains(candidate_id) && member.verify(&state.elders) {
                    state.to_mut().stable_set.apply(member);
                    let last_member = state.stable_set.last_member().unwrap();
                    let ord_idx = last_member.ord_idx + 1;
                    let sig = Sig::sign(id, (ord_idx, candidate_id));
                    o.send(src, Msg::JoinShare(ord_idx, candidate_id, sig, last_member));
                }
            }
            Msg::JoinShare(ord_idx, candidate_id, sig, last_member) => {
                let elders = state.elders.clone();
                let join_msg = (ord_idx, candidate_id);
                if id == candidate_id
                    && !state.stable_set.contains(id)
                    && sig.verify(src, &join_msg)
                    && last_member.verify(&state.elders)
                    && last_member.ord_idx + 1 == ord_idx
                {
                    let last_member_is_new = !state.stable_set.has_seen(last_member.id);
                    state.to_mut().stable_set.apply(last_member);

                    if (!state.joining_section_sig.is_empty()
                        && !state.joining_section_sig.contains_key(&ord_idx))
                        || last_member_is_new
                    {
                        let last_member = state.stable_set.last_member().unwrap();
                        o.broadcast(elders.iter(), &Msg::ReqJoin(id, last_member));
                    }

                    let section_sig = state
                        .to_mut()
                        .joining_section_sig
                        .entry(ord_idx)
                        .or_insert(SectionSig::new(elders.clone()));

                    section_sig.add_share(src, sig);

                    if section_sig.verify(&elders, &join_msg) {
                        o.broadcast(
                            &elders,
                            &Msg::Joined(ord_idx, candidate_id, section_sig.clone()),
                        )
                    }
                }
            }
            Msg::Joined(ord_idx, candidate_id, section_sig) => {
                if !state.stable_set.has_seen(candidate_id)
                    && section_sig.verify(&state.elders, &(ord_idx, candidate_id))
                {
                    state
                        .to_mut()
                        .stable_set
                        .add(ord_idx, candidate_id, section_sig.clone());

                    o.broadcast(
                        state.stable_set.iter(),
                        &Msg::Joined(ord_idx, candidate_id, section_sig),
                    );

                    for ((ord_idx, member), sig) in state.stable_set.iter_signed() {
                        o.send(candidate_id, Msg::Joined(*ord_idx, *member, sig.clone()));
                    }
                }
            }
        }
    }
}