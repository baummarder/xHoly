//! LEGACY: Xbox 360 XAM ordinals taken from Xenia. Not valid for Series X|S. Compiled only with feature legacy-xbox360.
//! XAM export table with **real Xbox 360 ordinals** (Phase 24).
//!
//! Retail titles do not call raw SVCs for XAM services — they import
//! `xam.xex` exports by ordinal. The ordinals below are taken from
//! Xenia's `xam_table.inc` (BSD-3-Clause), e.g. `XamInputGetState =
//! 0x191`, `XamUserGetSigninState = 0x210`. This module models the
//! export table and a dispatch layer with plausible return values;
//! it is not a full XAM implementation (no UI, profiles or storage).

/// `X_ERROR_SUCCESS`.
pub const X_ERROR_SUCCESS: u32 = 0;
/// `X_ERROR_DEVICE_NOT_CONNECTED`.
pub const X_ERROR_DEVICE_NOT_CONNECTED: u32 = 0x8007_0216;
/// `X_ERROR_INVALID_PARAMETER`.
pub const X_ERROR_INVALID_PARAMETER: u32 = 0x8007_0057;

/// One XAM export descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XamExport {
    /// Export name (documentation / name-based lookup).
    pub name: &'static str,
    /// Real Xbox 360 XAM ordinal.
    pub ordinal: u32,
    /// Category for the info panel.
    pub group: XamGroup,
}

/// Coarse XAM export groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XamGroup {
    /// Input / gamepad.
    Input,
    /// User profiles / sign-in.
    User,
    /// System information.
    System,
}

/// Verified XAM exports used by the dispatch table.
pub const XAM_EXPORTS: &[XamExport] = &[
    XamExport {
        name: "XamInputGetState",
        ordinal: 0x191,
        group: XamGroup::Input,
    },
    XamExport {
        name: "XamInputSetState",
        ordinal: 0x192,
        group: XamGroup::Input,
    },
    XamExport {
        name: "XamInputGetKeystroke",
        ordinal: 0x193,
        group: XamGroup::Input,
    },
    XamExport {
        name: "XamUserGetXUID",
        ordinal: 0x20A,
        group: XamGroup::User,
    },
    XamExport {
        name: "XamUserGetName",
        ordinal: 0x20E,
        group: XamGroup::User,
    },
    XamExport {
        name: "XamUserGetSigninState",
        ordinal: 0x210,
        group: XamGroup::User,
    },
    XamExport {
        name: "XamUserGetSigninInfo",
        ordinal: 0x227,
        group: XamGroup::User,
    },
    XamExport {
        name: "XamGetSystemVersion",
        ordinal: 0x282,
        group: XamGroup::System,
    },
];

/// Look up an export by ordinal.
pub fn by_ordinal(ordinal: u32) -> Option<&'static XamExport> {
    XAM_EXPORTS.iter().find(|e| e.ordinal == ordinal)
}

/// Look up an export by name.
pub fn by_name(name: &str) -> Option<&'static XamExport> {
    XAM_EXPORTS.iter().find(|e| e.name == name)
}

/// A connected gamepad's state (subset of `X_INPUT_STATE`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputState {
    /// Button bitmask.
    pub buttons: u16,
    /// Left stick X.
    pub left_x: i16,
    /// Left stick Y.
    pub left_y: i16,
}

/// Dispatch result for one XAM call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XamResult {
    /// HRESULT-style return code.
    pub code: u32,
    /// Optional output value (XUID, sign-in state, system version, ...).
    pub value: u64,
}

/// XAM ABI state: one connected controller + sign-in model.
#[derive(Debug, Clone)]
pub struct XamAbi {
    /// Whether a controller is connected on user 0.
    pub controller_connected: bool,
    /// Current input state, if connected.
    pub input: InputState,
    /// Number of signed-in users (0..4).
    pub signed_in_users: u32,
    /// System version reported by `XamGetSystemVersion`.
    pub system_version: u32,
    /// Calls dispatched so far.
    pub calls: u64,
}

impl Default for XamAbi {
    fn default() -> Self {
        Self {
            controller_connected: true,
            input: InputState::default(),
            signed_in_users: 1,
            system_version: 0x0002_0000,
            calls: 0,
        }
    }
}

impl XamAbi {
    /// Dispatch an export call with up to four arguments.
    pub fn call(&mut self, ordinal: u32, args: &[u32; 4]) -> XamResult {
        self.calls += 1;
        match ordinal {
            0x191 => {
                // XamInputGetState(user_index, flags, out_state, reserved)
                if !self.controller_connected || args[2] == 0 {
                    XamResult {
                        code: X_ERROR_DEVICE_NOT_CONNECTED,
                        value: 0,
                    }
                } else {
                    let bits = (self.input.buttons as u32)
                        | ((self.input.left_x as u16 as u32) << 16)
                        | ((self.input.left_y as u16 as u32) << 24);
                    XamResult {
                        code: X_ERROR_SUCCESS,
                        value: bits as u64,
                    }
                }
            }
            0x192 => XamResult {
                code: X_ERROR_SUCCESS,
                value: 0,
            }, // XamInputSetState
            0x193 => XamResult {
                code: X_ERROR_SUCCESS,
                value: 0,
            }, // XamInputGetKeystroke
            0x20A => XamResult {
                code: if args[0] < self.signed_in_users {
                    X_ERROR_SUCCESS
                } else {
                    X_ERROR_INVALID_PARAMETER
                },
                value: 0x0000_000A_0000_0001, // plausible XUID
            },
            0x20E => XamResult {
                code: X_ERROR_SUCCESS,
                value: b'X' as u64,
            },
            0x210 => XamResult {
                code: X_ERROR_SUCCESS,
                value: if args[0] < self.signed_in_users { 1 } else { 0 },
            },
            0x227 => XamResult {
                code: X_ERROR_SUCCESS,
                value: 1,
            },
            0x282 => XamResult {
                code: X_ERROR_SUCCESS,
                value: self.system_version as u64,
            },
            _ => XamResult {
                code: X_ERROR_INVALID_PARAMETER,
                value: 0,
            },
        }
    }

    /// Dispatch by export name (convenience for name-based import tables).
    pub fn call_name(&mut self, name: &str, args: &[u32; 4]) -> Option<XamResult> {
        by_name(name).map(|e| self.call(e.ordinal, args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinals_match_xenia() {
        assert_eq!(by_name("XamInputGetState").unwrap().ordinal, 0x191);
        assert_eq!(by_name("XamUserGetSigninState").unwrap().ordinal, 0x210);
        assert_eq!(by_name("XamGetSystemVersion").unwrap().ordinal, 0x282);
        assert_eq!(by_ordinal(0x227).unwrap().name, "XamUserGetSigninInfo");
    }

    #[test]
    fn input_get_state_needs_a_valid_output_pointer() {
        let mut abi = XamAbi::default();
        let r = abi.call(0x191, &[0, 0, 0x1000, 0]);
        assert_eq!(r.code, X_ERROR_SUCCESS);
        let r = abi.call(0x191, &[0, 0, 0, 0]);
        assert_eq!(r.code, X_ERROR_DEVICE_NOT_CONNECTED);
    }

    #[test]
    fn signin_state_follows_signed_in_users() {
        let mut abi = XamAbi::default();
        assert_eq!(
            abi.call_name("XamUserGetSigninState", &[0, 0, 0, 0])
                .unwrap()
                .value,
            1
        );
        assert_eq!(
            abi.call_name("XamUserGetSigninState", &[4, 0, 0, 0])
                .unwrap()
                .value,
            0
        );
    }

    #[test]
    fn unknown_export_returns_invalid_parameter() {
        let mut abi = XamAbi::default();
        assert_eq!(abi.call(0xDEAD, &[0; 4]).code, X_ERROR_INVALID_PARAMETER);
        assert_eq!(abi.calls, 1);
    }
}
