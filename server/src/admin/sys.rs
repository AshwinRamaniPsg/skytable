/*
 * Created on Tue Mar 29 2022
 *
 * This file is a part of Skytable
 * Skytable (formerly known as TerrabaseDB or Skybase) is a free and open-source
 * NoSQL database written by Sayan Nandan ("the Author") with the
 * vision to provide flexibility in data modelling without compromising
 * on performance, queryability or scalability.
 *
 * Copyright (c) 2022, Sayan Nandan <ohsayan@outlook.com>
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

use crate::corestore::booltable::BoolTable;
use crate::dbnet::connection::prelude::*;
use crate::protocol::{PROTOCOL_VERSION, PROTOCOL_VERSIONSTRING};
use ::libsky::VERSION;

const INFO: &[u8] = b"info";
const METRIC: &[u8] = b"metric";
const INFO_PROTOCOL: &[u8] = b"protocol";
const INFO_PROTOVER: &[u8] = b"protover";
const INFO_VERSION: &[u8] = b"version";
const METRIC_HEALTH: &[u8] = b"health";
const ERR_UNKNOWN_PROPERTY: &[u8] = b"!16\nunknown-property\n";
const ERR_UNKNOWN_METRIC: &[u8] = b"!14\nunknown-metric\n";

const HEALTH_TABLE: BoolTable<&str> = BoolTable::new("good", "critical");

action! {
    fn sys(_handle: &Corestore, con: &mut T, iter: ActionIter<'_>) {
        let mut iter = iter;
        ensure_boolean_or_aerr(iter.len() == 2)?;
        match unsafe { iter.next_lowercase_unchecked() }.as_ref() {
            INFO => sys_info(con, &mut iter).await,
            METRIC => sys_metric(con, &mut iter).await,
            _ => util::err(groups::UNKNOWN_ACTION),
        }
    }
    fn sys_info(con: &mut T, iter: &mut ActionIter<'_>) {
        match unsafe { iter.next_lowercase_unchecked() }.as_ref() {
            INFO_PROTOCOL => con.write_response(PROTOCOL_VERSIONSTRING).await?,
            INFO_PROTOVER => con.write_response(PROTOCOL_VERSION).await?,
            INFO_VERSION => con.write_response(VERSION).await?,
            _ => return util::err(ERR_UNKNOWN_PROPERTY),
        }
        Ok(())
    }
    fn sys_metric(con: &mut T, iter: &mut ActionIter<'_>) {
        match unsafe { iter.next_lowercase_unchecked() }.as_ref() {
            METRIC_HEALTH => {
                con.write_response(HEALTH_TABLE[registry::state_okay()]).await?
            }
            _ => return util::err(ERR_UNKNOWN_METRIC),
        }
        Ok(())
    }
}
