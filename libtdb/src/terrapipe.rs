/*
 * Created on Sat Jul 18 2020
 *
 * This file is a part of the source code for the Terrabase database
 * Copyright (c) 2020, Sayan Nandan <ohsayan at outlook dot com>
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 *
*/

//! # The Terrapipe protocol
//! This module implements primitives for the Terrapipe protocol
//!
//! Query and Response packet handling modules can be found in the `de`, `query`
//! and `response` modules from the crate root.

pub const ADDR: &'static str = "127.0.0.1:2003";

/// Default query metaline buffer size
pub const DEF_QMETALINE_BUFSIZE: usize = 44;
/// Default query metalayout buffer size
pub const DEF_QMETALAYOUT_BUFSIZE: usize = 576;
/// Default query dataframe buffer size
pub const DEF_QDATAFRAME_BUSIZE: usize = 4096;

/// Response codes returned by the server
#[derive(Debug, PartialEq)]
pub enum RespCodes {
    /// `0`: Okay (Empty Response) - use the `ResponseBuilder` for building
    /// responses that contain data
    Okay,
    /// `1`: Not Found
    NotFound,
    /// `2`: Overwrite Error
    OverwriteError,
    /// `3`: Action Error
    ActionError,
    /// `4`: Packet Error
    PacketError,
    /// `5`: Server Error
    ServerError,
    /// `6`: Some other error - the wrapped `String` will be returned in the response body.
    /// Just a note, this gets quite messy, especially when we're using it for deconding responses
    OtherError(Option<String>),
}

impl From<RespCodes> for u8 {
    fn from(rcode: RespCodes) -> u8 {
        use RespCodes::*;
        match rcode {
            Okay => 0,
            NotFound => 1,
            OverwriteError => 2,
            ActionError => 3,
            PacketError => 4,
            ServerError => 5,
            OtherError(_) => 6,
        }
    }
}

impl From<RespCodes> for char {
    fn from(rcode: RespCodes) -> char {
        use RespCodes::*;
        match rcode {
            Okay => '0',
            NotFound => '1',
            OverwriteError => '2',
            ActionError => '3',
            PacketError => '4',
            ServerError => '5',
            OtherError(_) => '6',
        }
    }
}

impl RespCodes {
    pub fn from_str(val: &str, extra: Option<String>) -> Option<Self> {
        use RespCodes::*;
        let res = match val.parse::<u8>() {
            Ok(val) => match val {
                0 => Okay,
                1 => NotFound,
                2 => OverwriteError,
                3 => ActionError,
                4 => PacketError,
                5 => ServerError,
                6 => OtherError(extra),
                _ => return None,
            },
            Err(_) => return None,
        };
        Some(res)
    }
    pub fn from_u8(val: u8, extra: Option<String>) -> Option<Self> {
        use RespCodes::*;
        let res = match val {
            0 => Okay,
            1 => NotFound,
            2 => OverwriteError,
            3 => ActionError,
            4 => PacketError,
            5 => ServerError,
            6 => OtherError(extra),
            _ => return None,
        };
        Some(res)
    }
    pub fn from_utf8(val: u8) -> Option<Self> {
        let result = match val.checked_sub(48) {
            Some(r) => r,
            None => return None,
        };
        if result > 6 {
            return None;
        }
        return RespCodes::from_u8(result, None);
    }
}

/// Representation of the query action type - pipelined or simple
#[derive(Debug, PartialEq)]
pub enum ActionType {
    Simple,
    Pipeline,
}