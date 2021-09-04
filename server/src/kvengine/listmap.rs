/*
 * Created on Tue Aug 31 2021
 *
 * This file is a part of Skytable
 * Skytable (formerly known as TerrabaseDB or Skybase) is a free and open-source
 * NoSQL database written by Sayan Nandan ("the Author") with the
 * vision to provide flexibility in data modelling without compromising
 * on performance, queryability or scalability.
 *
 * Copyright (c) 2021, Sayan Nandan <ohsayan@outlook.com>
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 *
*/

#![allow(dead_code)] // TODO(@ohsayan): Remove this once we're done

use super::SingleEncoder;
use crate::corestore::htable::Coremap;
use crate::corestore::Data;
use crate::resp::{TSYMBOL_BINARY, TSYMBOL_UNICODE};
use parking_lot::RwLock;

pub struct KVEListMap {
    encoded_id: bool,
    encoded_payload_element: bool,
    base: Coremap<Data, RwLock<Vec<Data>>>,
}

impl KVEListMap {
    /// Create a new KVEListMap. `Encoded ID == encoded key` and `encoded payload == encoded elements`
    pub fn new(encoded_id: bool, encoded_payload_element: bool) -> Self {
        Self {
            encoded_id,
            encoded_payload_element,
            base: Coremap::new(),
        }
    }
    /// Get an encoder instance for the payload elements
    pub fn get_payload_encoder(&self) -> SingleEncoder {
        s_encoder_booled!(self.encoded_payload_element)
    }
    /// Get an encoder instance for the ID
    pub fn get_id_encoder(&self) -> SingleEncoder {
        s_encoder_booled!(self.encoded_id)
    }
    pub fn encode_key<T: AsRef<[u8]>>(&self, val: T) -> bool {
        s_encoder!(self.encoded_id)(val.as_ref())
    }
    borrow_hash_fn! {
        pub fn {borrow: Data} len(self: &Self, key: &Q) -> Option<usize> {
            self.base.get(key).map(|v| v.read().len())
        }
    }
    pub fn add_list(&mut self, listname: Data) -> Option<bool> {
        if_cold! {
            if (self.encode_key(&listname)) {
                None
            } else {
                Some(self.base.true_if_insert(listname, RwLock::new(Vec::new())))
            }
        }
    }
}
