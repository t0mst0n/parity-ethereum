// Copyright 2015-2017 Parity Technologies (UK) Ltd.
// This file is part of Parity.

// Parity is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Parity is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Parity.  If not, see <http://www.gnu.org/licenses/>.

//! Pending request set.
//!
//! Stores pending requests and does timeout computation according to the rule
//! that only the earliest submitted request within the structure may time out.
//!
//! Whenever a request becomes the earliest, its timeout period begins at that moment.

use std::collections::{BTreeMap, HashMap};
use std::iter::FromIterator;

use request::Request;
use request::Requests;
use net::{timeout, ReqId};
use util::U256;

use time::{Duration, SteadyTime};

// Request set entry: requests + cost.
#[derive(Debug)]
struct Entry(Requests, U256);

/// Request set.
#[derive(Debug)]
pub struct RequestSet {
	counter: u64,
	cumulative_cost: U256,
	base: Option<SteadyTime>,
	ids: HashMap<ReqId, u64>,
	reqs: BTreeMap<u64, Entry>,
}

impl Default for RequestSet {
	fn default() -> Self {
		RequestSet {
			counter: 0,
			cumulative_cost: 0.into(),
			base: None,
			ids: HashMap::new(),
			reqs: BTreeMap::new(),
		}
	}
}

impl RequestSet {
	/// Push requests onto the stack.
	pub fn insert(&mut self, req_id: ReqId, req: Requests, cost: U256, now: SteadyTime) {
		let counter = self.counter;
		self.cumulative_cost = self.cumulative_cost + cost;

		self.ids.insert(req_id, counter);
		self.reqs.insert(counter, Entry(req, cost));

		if self.reqs.keys().next().map_or(true, |x| *x == counter) {
			self.base = Some(now);
		}

		self.counter += 1;
	}

	/// Remove a set of requests from the stack.
	pub fn remove(&mut self, req_id: &ReqId, now: SteadyTime) -> Option<Requests> {
		let id = match self.ids.remove(&req_id) {
			Some(id) => id,
			None => return None,
		};

		let Entry(req, cost) = self.reqs.remove(&id).expect("entry in `ids` implies entry in `reqs`; qed");

		match self.reqs.keys().next() {
			Some(k) if *k > id => self.base = Some(now),
			None => self.base = None,
			_ => {}
		}

		self.cumulative_cost = self.cumulative_cost - cost;
		Some(req)
	}

	/// Check for timeout against the given time. Returns true if
	/// has timed out, false otherwise.
	pub fn check_timeout(&self, now: SteadyTime) -> bool {
		let base = match self.base.as_ref().cloned() {
			Some(base) => base,
			None => return false,
		};

		let first_req = self.reqs.values().next()
			.expect("base existing implies `reqs` non-empty; qed");

		base + compute_timeout(&first_req.0) <= now
	}

	/// Collect all pending request ids.
	pub fn collect_ids<F>(&self) -> F where F: FromIterator<ReqId> {
		self.ids.keys().cloned().collect()
	}

	/// Number of requests in the set.
	pub fn len(&self) -> usize {
		self.ids.len()
	}

	/// Whether the set is empty.
	pub fn is_empty(&self) -> bool { self.len() == 0 }

	/// The cumulative cost of all requests in the set.
	pub fn cumulative_cost(&self) -> U256 { self.cumulative_cost }
}

// helper to calculate timeout for a specific set of requests.
// it's a base amount + some amount per request.
fn compute_timeout(reqs: &Requests) -> Duration {
	Duration::milliseconds(reqs.requests().iter().fold(timeout::BASE, |tm, req| {
		tm + match *req {
			Request::Headers(_) => timeout::HEADERS,
			Request::HeaderProof(_) => timeout::HEADER_PROOF,
			Request::Receipts(_) => timeout::RECEIPT,
			Request::Body(_) => timeout::BODY,
			Request::Account(_) => timeout::PROOF,
			Request::Storage(_) => timeout::PROOF,
			Request::Code(_) => timeout::CONTRACT_CODE,
			Request::Execution(_) => timeout::TRANSACTION_PROOF,
		}
	}))
}

#[cfg(test)]
mod tests {
	use net::ReqId;
	use request::RequestBuilder;
	use time::{SteadyTime, Duration};
	use super::{RequestSet, compute_timeout};

	#[test]
	fn multi_timeout() {
		let test_begin = SteadyTime::now();
		let mut req_set = RequestSet::default();

		let the_req = RequestBuilder::default().build();
		let req_time = compute_timeout(&the_req);
		req_set.insert(ReqId(0), the_req.clone(), 0.into(), test_begin);
		req_set.insert(ReqId(1), the_req, 0.into(), test_begin + Duration::seconds(1));

		assert_eq!(req_set.base, Some(test_begin));

		let test_end = test_begin + req_time;
		assert!(req_set.check_timeout(test_end));

		req_set.remove(&ReqId(0), test_begin + Duration::seconds(1)).unwrap();
		assert!(!req_set.check_timeout(test_end));
		assert!(req_set.check_timeout(test_end + Duration::seconds(1)));
	}

	#[test]
	fn cumulative_cost() {
		let the_req = RequestBuilder::default().build();
		let test_begin = SteadyTime::now();
		let test_end = test_begin + Duration::seconds(1);
		let mut req_set = RequestSet::default();

		for i in 0..5 {
			req_set.insert(ReqId(i), the_req.clone(), 1.into(), test_begin);
			assert_eq!(req_set.cumulative_cost, (i + 1).into());
		}

		for i in (0..5).rev() {
			assert!(req_set.remove(&ReqId(i), test_end).is_some());
			assert_eq!(req_set.cumulative_cost, i.into());
		}
	}
}