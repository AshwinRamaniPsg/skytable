/*
 * Created on Wed Sep 15 2021
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

use super::{writer, OKAY_BADIDX_NIL_NLUT};
use crate::corestore::Data;
use crate::dbnet::connection::prelude::*;
use crate::util::compiler;

const CLEAR: &[u8] = "CLEAR".as_bytes();
const PUSH: &[u8] = "PUSH".as_bytes();
const REMOVE: &[u8] = "REMOVE".as_bytes();
const INSERT: &[u8] = "INSERT".as_bytes();
const POP: &[u8] = "POP".as_bytes();

action! {
    /// Handle `LMOD` queries
    /// ## Syntax
    /// - `LMOD <mylist> push <value>`
    /// - `LMOD <mylist> pop <optional idx>`
    /// - `LMOD <mylist> insert <index> <value>`
    /// - `LMOD <mylist> remove <index>`
    /// - `LMOD <mylist> clear`
    fn lmod(handle: &Corestore, con: &mut T, mut act: ActionIter<'a>) {
        ensure_length(act.len(), |len| len > 1)?;
        let listmap = handle.get_table_with::<KVEList>()?;
        // get the list name
        let listname = unsafe { act.next_unchecked() };
        macro_rules! get_numeric_count {
            () => {
                match unsafe { String::from_utf8_lossy(act.next_unchecked()) }.parse::<usize>() {
                    Ok(int) => int,
                    Err(_) => return conwrite!(con, groups::WRONGTYPE_ERR),
                }
            };
        }
        // now let us see what we need to do
        match unsafe { act.next_uppercase_unchecked() }.as_ref() {
            CLEAR => {
                ensure_length(act.len(), |len| len == 0)?;
                let list = match listmap.get_inner_ref().get(listname) {
                    Some(l) => l,
                    _ => return conwrite!(con, groups::NIL),
                };
                let okay = if registry::state_okay() {
                    list.write().clear();
                    groups::OKAY
                } else {
                    groups::SERVER_ERR
                };
                conwrite!(con, okay)?;
            }
            PUSH => {
                ensure_boolean_or_aerr(!act.is_empty())?;
                let list = match listmap.get_inner_ref().get(listname) {
                    Some(l) => l,
                    _ => return conwrite!(con, groups::NIL),
                };
                let venc_ok = listmap.get_val_encoder();
                let ret = if compiler::likely(act.as_ref().all(venc_ok)) {
                    if registry::state_okay() {
                        list.write().extend(act.map(Data::copy_from_slice));
                        groups::OKAY
                    } else {
                        groups::SERVER_ERR
                    }
                } else {
                    groups::ENCODING_ERROR
                };
                conwrite!(con, ret)?;
            }
            REMOVE => {
                ensure_length(act.len(), |len| len == 1)?;
                let idx_to_remove = get_numeric_count!();
                if registry::state_okay() {
                    let maybe_value = listmap.get_inner_ref().get(listname).map(|list| {
                        let mut wlock = list.write();
                        if idx_to_remove < wlock.len() {
                            wlock.remove(idx_to_remove);
                            true
                        } else {
                            false
                        }
                    });
                    conwrite!(con, OKAY_BADIDX_NIL_NLUT[maybe_value])?;
                } else {
                    conwrite!(con, groups::SERVER_ERR)?;
                }
            }
            INSERT => {
                ensure_length(act.len(), |len| len == 2)?;
                let idx_to_insert_at = get_numeric_count!();
                let bts = unsafe { act.next_unchecked() };
                let ret = if compiler::likely(listmap.is_val_ok(bts)) {
                    if registry::state_okay() {
                        // okay state, good to insert
                        let maybe_insert = match listmap.get(listname) {
                            Ok(lst) => lst.map(|list| {
                                let mut wlock = list.write();
                                if idx_to_insert_at < wlock.len() {
                                    // we can insert
                                    wlock.insert(idx_to_insert_at, Data::copy_from_slice(bts));
                                    true
                                } else {
                                    // oops, out of bounds
                                    false
                                }
                            }),
                            Err(()) => return conwrite!(con, groups::ENCODING_ERROR),
                        };
                        OKAY_BADIDX_NIL_NLUT[maybe_insert]
                    } else {
                        // flush broken; server err
                        groups::SERVER_ERR
                    }
                } else {
                    // encoding failed, uh
                    groups::ENCODING_ERROR
                };
                conwrite!(con, ret)?;
            }
            POP => {
                ensure_length(act.len(), |len| len < 2)?;
                let idx = if act.len() == 1 {
                    // we have an idx
                    Some(get_numeric_count!())
                } else {
                    // no idx
                    None
                };
                if registry::state_okay() {
                    let maybe_pop = match listmap.get(listname) {
                        Ok(lst) => lst.map(|list| {
                            let mut wlock = list.write();
                            if let Some(idx) = idx {
                                if idx < wlock.len() {
                                    // so we can pop
                                    Some(wlock.remove(idx))
                                } else {
                                    None
                                }
                            } else {
                                wlock.pop()
                            }
                        }),
                        Err(()) => return conwrite!(con, groups::ENCODING_ERROR),
                    };
                    match maybe_pop {
                        Some(Some(val)) => {
                            unsafe {
                                writer::write_raw_mono(con, listmap.get_value_tsymbol(), &val).await?;
                            }
                        }
                        Some(None) => {
                            conwrite!(con, groups::LISTMAP_BAD_INDEX)?;
                        }
                        None => conwrite!(con, groups::NIL)?,
                    }
                } else {
                    conwrite!(con, groups::SERVER_ERR)?;
                }
            }
            _ => conwrite!(con, groups::UNKNOWN_ACTION)?,
        }
        Ok(())
    }
}
