/*
 * Created on Tue Jul 27 2021
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

use super::parser;
use super::parser::VALID_CONTAINER_NAME;
use crate::corestore::memstore::ObjectID;
use crate::dbnet::connection::prelude::*;
use crate::kvengine::encoding;
use crate::registry;
use core::str;

pub const TABLE: &[u8] = "TABLE".as_bytes();
pub const KEYSPACE: &[u8] = "KEYSPACE".as_bytes();
const VOLATILE: &[u8] = "volatile".as_bytes();
const FORCE_REMOVE: &[u8] = "force".as_bytes();

action! {
    /// Handle `create table <tableid> <model>(args)` and `create keyspace <ksid>`
    /// like queries
    fn create(handle: &Corestore, con: &'a mut T, mut act: ActionIter<'a>) {
        // minlength is 2 (create has already been checked)
        ensure_length(act.len(), |size| size > 1)?;
        let mut create_what = unsafe { act.next().unsafe_unwrap() }.to_vec();
        create_what.make_ascii_uppercase();
        match create_what.as_ref() {
            TABLE => create_table(handle, con, act).await?,
            KEYSPACE => create_keyspace(handle, con, act).await?,
            _ => {
                con.write_response(responses::groups::UNKNOWN_DDL_QUERY)
                    .await?;
            }
        }
        Ok(())
    }

    /// Handle `drop table <tableid>` and `drop keyspace <ksid>`
    /// like queries
    fn ddl_drop(handle: &Corestore, con: &'a mut T, mut act: ActionIter<'a>) {
        // minlength is 2 (create has already been checked)
        ensure_length(act.len(), |size| size > 1)?;
        let mut create_what = unsafe { act.next().unsafe_unwrap() }.to_vec();
        create_what.make_ascii_uppercase();
        match create_what.as_ref() {
            TABLE => drop_table(handle, con, act).await?,
            KEYSPACE => drop_keyspace(handle, con, act).await?,
            _ => {
                con.write_response(responses::groups::UNKNOWN_DDL_QUERY)
                    .await?;
            }
        }
        Ok(())
    }

    /// We should have `<tableid> <model>(args) properties`
    fn create_table(handle: &Corestore, con: &'a mut T, mut act: ActionIter<'a>) {
        ensure_length(act.len(), |size| size > 1 && size < 4)?;
        let table_name = unsafe { act.next().unsafe_unwrap() };
        let model_name = unsafe { act.next().unsafe_unwrap() };
        let (table_entity, model_code) = parser::parse_table_args(table_name, model_name)?;
        let is_volatile = match act.next() {
            Some(maybe_volatile) => {
                ensure_cond_or_err(maybe_volatile.eq(VOLATILE), responses::groups::UNKNOWN_PROPERTY)?;
                true
            }
            None => false,
        };
        if registry::state_okay() {
            handle.create_table(table_entity, model_code, is_volatile)?;
            con.write_response(responses::groups::OKAY).await?;
        } else {
            conwrite!(con, responses::groups::SERVER_ERR)?;
        }
        Ok(())
    }

    /// We should have `<ksid>`
    fn create_keyspace(handle: &Corestore, con: &'a mut T, mut act: ActionIter<'a>) {
        ensure_length(act.len(), |len| len == 1)?;
        match act.next() {
            Some(ksid) => {
                ensure_cond_or_err(encoding::is_utf8(&ksid), responses::groups::ENCODING_ERROR)?;
                let ksid_str = unsafe { str::from_utf8_unchecked(ksid) };
                ensure_cond_or_err(VALID_CONTAINER_NAME.is_match(ksid_str), responses::groups::BAD_EXPRESSION)?;
                ensure_cond_or_err(ksid.len() < 64, responses::groups::CONTAINER_NAME_TOO_LONG)?;
                let ksid = unsafe { ObjectID::from_slice(ksid_str) };
                if registry::state_okay() {
                    handle.create_keyspace(ksid)?;
                    con.write_response(responses::groups::OKAY).await?
                } else {
                    conwrite!(con, responses::groups::SERVER_ERR)?;
                }
            }
            None => con.write_response(responses::groups::ACTION_ERR).await?,
        }
        Ok(())
    }

    /// Drop a table (`<tblid>` only)
    fn drop_table(handle: &Corestore, con: &'a mut T, mut act: ActionIter<'a>) {
        ensure_length(act.len(), |size| size == 1)?;
        match act.next() {
            Some(eg) => {
                let entity_group = parser::Entity::from_slice(eg)?;
                if registry::state_okay() {
                    handle.drop_table(entity_group)?;
                    con.write_response(responses::groups::OKAY).await?;
                } else {
                    conwrite!(con, responses::groups::SERVER_ERR)?;
                }
            },
            None => con.write_response(responses::groups::ACTION_ERR).await?,
        }
        Ok(())
    }

    /// Drop a keyspace (`<ksid>` only)
    fn drop_keyspace(handle: &Corestore, con: &'a mut T, mut act: ActionIter<'a>) {
        ensure_length(act.len(), |size| size == 1)?;
        match act.next() {
            Some(ksid) => {
                ensure_cond_or_err(ksid.len() < 64, responses::groups::CONTAINER_NAME_TOO_LONG)?;
                let force_remove = match act.next() {
                    Some(bts) if bts.eq(FORCE_REMOVE) => true,
                    None => false,
                    _ => {
                        return util::err(responses::groups::UNKNOWN_ACTION);
                    }
                };
                if registry::state_okay() {
                    let objid = unsafe {ObjectID::from_slice(ksid)};
                    let result = if force_remove {
                        handle.force_drop_keyspace(objid)
                    } else {
                        handle.drop_keyspace(objid)
                    };
                    result?;
                    con.write_response(responses::groups::OKAY).await?;
                } else {
                    conwrite!(con, responses::groups::SERVER_ERR)?;
                }
            },
            None => con.write_response(responses::groups::ACTION_ERR).await?,
        }
        Ok(())
    }
}
