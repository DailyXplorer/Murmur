//! Where the Mac plays sound during a meeting, and so whether the microphone
//! may hear the remote participants. Headphones keep them out of it; the
//! built-in speakers, a display or a Bluetooth speaker do not, and the echo
//! filter (`bleed`) then has work to do. Unknown routes count as speakers,
//! which is what the filter assumed before routes were read.

/// What Core Audio says about the default output device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteFacts {
    /// `kAudioDevicePropertyTransportType`.
    pub transport: u32,
    /// `kAudioDevicePropertyDataSource` on the output, when the device has
    /// one: the built-in output says whether a jack is plugged.
    pub data_source: Option<u32>,
    /// The device also has input streams, as a headset does.
    pub has_input: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputRoute {
    Headphones,
    Speakers,
    /// A virtual or aggregate device, a built-in output that does not say,
    /// or no output at all.
    Unknown,
}

const fn code(name: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*name)
}

const BUILT_IN: u32 = code(b"bltn");
const BLUETOOTH: u32 = code(b"blue");
const BLUETOOTH_LE: u32 = code(b"blea");
const USB: u32 = code(b"usb ");
const HDMI: u32 = code(b"hdmi");
const DISPLAY_PORT: u32 = code(b"dprt");
const AIRPLAY: u32 = code(b"airp");
const INTERNAL_SPEAKER: u32 = code(b"ispk");
const HEADPHONES_JACK: u32 = code(b"hdpn");

impl OutputRoute {
    /// AirPods and USB headsets have a microphone of their own; Bluetooth
    /// and USB speakers have none. Built-in output says which of its
    /// sources plays. Displays and AirPlay play into the room.
    pub fn of(facts: Option<RouteFacts>) -> Self {
        let Some(facts) = facts else {
            return Self::Unknown;
        };
        match facts.transport {
            BUILT_IN => match facts.data_source {
                Some(HEADPHONES_JACK) => Self::Headphones,
                Some(INTERNAL_SPEAKER) => Self::Speakers,
                _ => Self::Unknown,
            },
            BLUETOOTH | BLUETOOTH_LE | USB if facts.has_input => Self::Headphones,
            BLUETOOTH | BLUETOOTH_LE | USB | HDMI | DISPLAY_PORT | AIRPLAY => Self::Speakers,
            _ => Self::Unknown,
        }
    }

    /// The route of a meeting that played on `self` and then on `other`:
    /// headphones only while every moment was, speakers once any was.
    pub fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Headphones, Self::Headphones) => Self::Headphones,
            (Self::Speakers, _) | (_, Self::Speakers) => Self::Speakers,
            _ => Self::Unknown,
        }
    }

    /// The name kept in the meetings database.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Headphones => "headphones",
            Self::Speakers => "speakers",
            Self::Unknown => "unknown",
        }
    }

    /// Reads back `as_str`; a meeting recorded before routes were read is
    /// `Unknown`.
    pub fn parse(name: Option<&str>) -> Self {
        match name {
            Some("headphones") => Self::Headphones,
            Some("speakers") => Self::Speakers,
            _ => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(transport: &[u8; 4], data_source: Option<&[u8; 4]>, has_input: bool) -> RouteFacts {
        RouteFacts {
            transport: code(transport),
            data_source: data_source.map(code),
            has_input,
        }
    }

    #[test]
    fn each_output_is_told_apart_by_its_transport_source_and_inputs() {
        use OutputRoute::*;
        let cases = [
            (facts(b"bltn", Some(b"ispk"), false), Speakers),
            (facts(b"bltn", Some(b"hdpn"), false), Headphones),
            (facts(b"bltn", None, false), Unknown),
            (facts(b"bltn", Some(b"lout"), false), Unknown),
            // AirPods and a Bluetooth headset.
            (facts(b"blue", None, true), Headphones),
            (facts(b"blea", None, true), Headphones),
            // A Bluetooth speaker.
            (facts(b"blue", None, false), Speakers),
            // A USB headset, then USB speakers.
            (facts(b"usb ", None, true), Headphones),
            (facts(b"usb ", None, false), Speakers),
            (facts(b"hdmi", None, false), Speakers),
            (facts(b"dprt", None, false), Speakers),
            (facts(b"airp", None, false), Speakers),
            // A call app's virtual device, an aggregate device.
            (facts(b"virt", None, true), Unknown),
            (facts(b"grup", None, true), Unknown),
            (facts(b"\0\0\0\0", None, false), Unknown),
        ];
        for (facts, route) in cases {
            assert_eq!(OutputRoute::of(Some(facts)), route, "{facts:?}");
        }
        assert_eq!(OutputRoute::of(None), Unknown);
    }

    #[test]
    fn a_meeting_is_on_headphones_only_while_every_moment_was() {
        use OutputRoute::*;
        assert_eq!(Headphones.and(Headphones), Headphones);
        assert_eq!(Headphones.and(Speakers), Speakers);
        assert_eq!(Speakers.and(Headphones), Speakers);
        assert_eq!(Headphones.and(Unknown), Unknown);
        assert_eq!(Unknown.and(Speakers), Speakers);
        assert_eq!(Unknown.and(Unknown), Unknown);
    }

    #[test]
    fn the_route_reads_back_as_it_was_kept() {
        for route in [
            OutputRoute::Headphones,
            OutputRoute::Speakers,
            OutputRoute::Unknown,
        ] {
            assert_eq!(OutputRoute::parse(Some(route.as_str())), route);
        }
        assert_eq!(OutputRoute::parse(None), OutputRoute::Unknown);
    }
}
