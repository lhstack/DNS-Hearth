//! DNS message parsing and structures
//!
//! Provides DnsQuery and DnsResponse structures for DNS message handling,
//! supporting all DNS record types (A, AAAA, CNAME, MX, TXT, PTR, NS, SOA, SRV).

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use hickory_proto::op::{Message, MessageType, OpCode, ResponseCode};
use hickory_proto::rr::{Name, RData, Record, RecordType as TrustRecordType};
use hickory_proto::serialize::binary::{BinDecodable, BinEncodable};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// DNS-specific errors
#[derive(Error, Debug)]
pub enum DnsError {
    #[error("Failed to parse DNS message: {0}")]
    ParseError(String),

    #[error("Failed to encode DNS message: {0}")]
    EncodeError(String),

    #[error("Invalid record type: {0}")]
    InvalidRecordType(String),

    #[error("Invalid domain name: {0}")]
    InvalidDomainName(String),

    #[error("Query failed: {0}")]
    #[allow(dead_code)]
    QueryFailed(String),
}

/// Supported DNS record types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum RecordType {
    A,
    AAAA,
    CNAME,
    MX,
    TXT,
    PTR,
    NS,
    SOA,
    SRV,
}

impl RecordType {
    /// Convert to trust-dns RecordType
    pub fn to_trust_dns(&self) -> TrustRecordType {
        match self {
            RecordType::A => TrustRecordType::A,
            RecordType::AAAA => TrustRecordType::AAAA,
            RecordType::CNAME => TrustRecordType::CNAME,
            RecordType::MX => TrustRecordType::MX,
            RecordType::TXT => TrustRecordType::TXT,
            RecordType::PTR => TrustRecordType::PTR,
            RecordType::NS => TrustRecordType::NS,
            RecordType::SOA => TrustRecordType::SOA,
            RecordType::SRV => TrustRecordType::SRV,
        }
    }

    /// Convert from trust-dns RecordType
    pub fn from_trust_dns(rt: TrustRecordType) -> Option<Self> {
        match rt {
            TrustRecordType::A => Some(RecordType::A),
            TrustRecordType::AAAA => Some(RecordType::AAAA),
            TrustRecordType::CNAME => Some(RecordType::CNAME),
            TrustRecordType::MX => Some(RecordType::MX),
            TrustRecordType::TXT => Some(RecordType::TXT),
            TrustRecordType::PTR => Some(RecordType::PTR),
            TrustRecordType::NS => Some(RecordType::NS),
            TrustRecordType::SOA => Some(RecordType::SOA),
            TrustRecordType::SRV => Some(RecordType::SRV),
            _ => None,
        }
    }

    /// Get all supported record types
    #[allow(dead_code)]
    pub fn all() -> &'static [RecordType] {
        &[
            RecordType::A,
            RecordType::AAAA,
            RecordType::CNAME,
            RecordType::MX,
            RecordType::TXT,
            RecordType::PTR,
            RecordType::NS,
            RecordType::SOA,
            RecordType::SRV,
        ]
    }
}

impl fmt::Display for RecordType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecordType::A => write!(f, "A"),
            RecordType::AAAA => write!(f, "AAAA"),
            RecordType::CNAME => write!(f, "CNAME"),
            RecordType::MX => write!(f, "MX"),
            RecordType::TXT => write!(f, "TXT"),
            RecordType::PTR => write!(f, "PTR"),
            RecordType::NS => write!(f, "NS"),
            RecordType::SOA => write!(f, "SOA"),
            RecordType::SRV => write!(f, "SRV"),
        }
    }
}

impl FromStr for RecordType {
    type Err = DnsError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "A" => Ok(RecordType::A),
            "AAAA" => Ok(RecordType::AAAA),
            "CNAME" => Ok(RecordType::CNAME),
            "MX" => Ok(RecordType::MX),
            "TXT" => Ok(RecordType::TXT),
            "PTR" => Ok(RecordType::PTR),
            "NS" => Ok(RecordType::NS),
            "SOA" => Ok(RecordType::SOA),
            "SRV" => Ok(RecordType::SRV),
            _ => Err(DnsError::InvalidRecordType(s.to_string())),
        }
    }
}

/// DNS query structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsQuery {
    /// Query ID for matching responses
    pub id: u16,
    /// Domain name being queried
    pub name: String,
    /// Record type being queried
    pub record_type: RecordType,
    /// Whether recursion is desired
    pub recursion_desired: bool,
    /// EDNS(0) advertised UDP payload size, when the client sent an OPT record.
    ///
    /// `None` means the client did not use EDNS, so responses must respect the
    /// classic 512 byte UDP limit from RFC 1035.
    #[serde(default)]
    pub udp_payload_size: Option<u16>,
}

impl DnsQuery {
    /// Create a new DNS query
    pub fn new(name: impl Into<String>, record_type: RecordType) -> Self {
        Self {
            id: rand_id(),
            name: name.into(),
            record_type,
            recursion_desired: true,
            udp_payload_size: None,
        }
    }

    /// Create a DNS query with a specific ID
    pub fn with_id(id: u16, name: impl Into<String>, record_type: RecordType) -> Self {
        Self {
            id,
            name: name.into(),
            record_type,
            recursion_desired: true,
            udp_payload_size: None,
        }
    }

    /// Parse a DNS query from raw bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, DnsError> {
        let message = Message::from_bytes(data).map_err(|e| DnsError::ParseError(e.to_string()))?;

        let query = message
            .queries()
            .first()
            .ok_or_else(|| DnsError::ParseError("No query in message".to_string()))?;

        let record_type = RecordType::from_trust_dns(query.query_type())
            .ok_or_else(|| DnsError::InvalidRecordType(query.query_type().to_string()))?;

        Ok(Self {
            id: message.id(),
            name: query.name().to_string().trim_end_matches('.').to_string(),
            record_type,
            recursion_desired: message.recursion_desired(),
            udp_payload_size: message.extensions().as_ref().map(|edns| edns.max_payload()),
        })
    }

    /// Encode the DNS query to raw bytes
    pub fn to_bytes(&self) -> Result<Vec<u8>, DnsError> {
        let name =
            Name::from_str(&self.name).map_err(|e| DnsError::InvalidDomainName(e.to_string()))?;

        let mut message = Message::new();
        message.set_id(self.id);
        message.set_message_type(MessageType::Query);
        message.set_op_code(OpCode::Query);
        message.set_recursion_desired(self.recursion_desired);

        message.add_query(hickory_proto::op::Query::query(
            name,
            self.record_type.to_trust_dns(),
        ));

        // Preserve the client's EDNS(0) buffer advertisement so the upstream can
        // return large answers instead of truncating them.
        if let Some(payload_size) = self.udp_payload_size {
            let mut edns = hickory_proto::op::Edns::new();
            edns.set_max_payload(payload_size);
            message.set_edns(edns);
        }

        message
            .to_bytes()
            .map_err(|e| DnsError::EncodeError(e.to_string()))
    }

    /// Maximum response size this client can accept over UDP.
    ///
    /// RFC 6891 lets clients advertise a larger buffer via EDNS(0); without it
    /// the classic 512 byte limit applies.
    pub fn max_udp_response_size(&self) -> usize {
        const CLASSIC_UDP_LIMIT: usize = 512;
        const MIN_EDNS_LIMIT: usize = 512;
        const MAX_EDNS_LIMIT: usize = 4096;

        match self.udp_payload_size {
            None => CLASSIC_UDP_LIMIT,
            Some(advertised) => (advertised as usize).clamp(MIN_EDNS_LIMIT, MAX_EDNS_LIMIT),
        }
    }
}

/// Transaction ID from a raw DNS message header, if it parses.
///
/// The UDP client uses this to drop datagrams that do not belong to the query
/// it sent: without the check any host able to guess the source port could
/// inject a forged answer.
pub fn message_transaction_id(data: &[u8]) -> Option<u16> {
    Message::from_bytes(data).ok().map(|m| m.id())
}

/// Whether a raw DNS message has the TC (truncated) header bit set.
///
/// Used by the UDP upstream client to decide whether the answer must be
/// re-fetched over TCP per RFC 1035 section 4.2.1.
pub fn message_is_truncated(data: &[u8]) -> bool {
    Message::from_bytes(data)
        .map(|m| m.truncated())
        .unwrap_or(false)
}

/// Generate a random query ID
fn rand_id() -> u16 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos % 65536) as u16
}

/// A single DNS record in a response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsRecordData {
    /// Record name
    pub name: String,
    /// Record type
    pub record_type: RecordType,
    /// Record value (formatted as string)
    pub value: String,
    /// Time to live in seconds
    pub ttl: u32,
    /// Priority (for MX and SRV records)
    pub priority: Option<u16>,
}

impl DnsRecordData {
    /// Create a new A record
    pub fn a(name: impl Into<String>, ip: Ipv4Addr, ttl: u32) -> Self {
        Self {
            name: name.into(),
            record_type: RecordType::A,
            value: ip.to_string(),
            ttl,
            priority: None,
        }
    }

    /// Create a new AAAA record
    pub fn aaaa(name: impl Into<String>, ip: Ipv6Addr, ttl: u32) -> Self {
        Self {
            name: name.into(),
            record_type: RecordType::AAAA,
            value: ip.to_string(),
            ttl,
            priority: None,
        }
    }

    /// Create a new CNAME record
    pub fn cname(name: impl Into<String>, target: impl Into<String>, ttl: u32) -> Self {
        Self {
            name: name.into(),
            record_type: RecordType::CNAME,
            value: target.into(),
            ttl,
            priority: None,
        }
    }

    /// Create a new MX record
    pub fn mx(
        name: impl Into<String>,
        exchange: impl Into<String>,
        priority: u16,
        ttl: u32,
    ) -> Self {
        Self {
            name: name.into(),
            record_type: RecordType::MX,
            value: exchange.into(),
            ttl,
            priority: Some(priority),
        }
    }

    /// Create a new TXT record
    pub fn txt(name: impl Into<String>, text: impl Into<String>, ttl: u32) -> Self {
        Self {
            name: name.into(),
            record_type: RecordType::TXT,
            value: text.into(),
            ttl,
            priority: None,
        }
    }

    /// Create a new PTR record
    pub fn ptr(name: impl Into<String>, target: impl Into<String>, ttl: u32) -> Self {
        Self {
            name: name.into(),
            record_type: RecordType::PTR,
            value: target.into(),
            ttl,
            priority: None,
        }
    }

    /// Create a new NS record
    pub fn ns(name: impl Into<String>, nameserver: impl Into<String>, ttl: u32) -> Self {
        Self {
            name: name.into(),
            record_type: RecordType::NS,
            value: nameserver.into(),
            ttl,
            priority: None,
        }
    }
}

/// DNS response codes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DnsResponseCode {
    NoError,
    FormErr,
    ServFail,
    NxDomain,
    NotImp,
    Refused,
    Other(u16),
}

impl DnsResponseCode {
    /// Convert from trust-dns ResponseCode
    pub fn from_trust_dns(code: ResponseCode) -> Self {
        match code {
            ResponseCode::NoError => DnsResponseCode::NoError,
            ResponseCode::FormErr => DnsResponseCode::FormErr,
            ResponseCode::ServFail => DnsResponseCode::ServFail,
            ResponseCode::NXDomain => DnsResponseCode::NxDomain,
            ResponseCode::NotImp => DnsResponseCode::NotImp,
            ResponseCode::Refused => DnsResponseCode::Refused,
            _ => DnsResponseCode::Other(code.into()),
        }
    }

    /// Convert to trust-dns ResponseCode
    pub fn to_trust_dns(&self) -> ResponseCode {
        match self {
            DnsResponseCode::NoError => ResponseCode::NoError,
            DnsResponseCode::FormErr => ResponseCode::FormErr,
            DnsResponseCode::ServFail => ResponseCode::ServFail,
            DnsResponseCode::NxDomain => ResponseCode::NXDomain,
            DnsResponseCode::NotImp => ResponseCode::NotImp,
            DnsResponseCode::Refused => ResponseCode::Refused,
            DnsResponseCode::Other(code) => {
                let high = ((*code >> 8) & 0xFF) as u8;
                let low = (*code & 0xFF) as u8;
                ResponseCode::from(high, low)
            }
        }
    }
}

impl fmt::Display for DnsResponseCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DnsResponseCode::NoError => write!(f, "NOERROR"),
            DnsResponseCode::FormErr => write!(f, "FORMERR"),
            DnsResponseCode::ServFail => write!(f, "SERVFAIL"),
            DnsResponseCode::NxDomain => write!(f, "NXDOMAIN"),
            DnsResponseCode::NotImp => write!(f, "NOTIMP"),
            DnsResponseCode::Refused => write!(f, "REFUSED"),
            DnsResponseCode::Other(code) => write!(f, "OTHER({})", code),
        }
    }
}

/// DNS response structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsResponse {
    /// Response ID (matches query ID)
    pub id: u16,
    /// Response code
    pub response_code: DnsResponseCode,
    /// Whether this is an authoritative answer
    pub authoritative: bool,
    /// Whether recursion is available
    pub recursion_available: bool,
    /// Answer records
    pub answers: Vec<DnsRecordData>,
    /// Authority records
    pub authority: Vec<DnsRecordData>,
    /// Additional records
    pub additional: Vec<DnsRecordData>,
}

impl DnsResponse {
    /// Create a new empty response
    pub fn new(id: u16) -> Self {
        Self {
            id,
            response_code: DnsResponseCode::NoError,
            authoritative: false,
            recursion_available: true,
            answers: Vec::new(),
            authority: Vec::new(),
            additional: Vec::new(),
        }
    }

    /// Create an NXDOMAIN response
    pub fn nxdomain(id: u16) -> Self {
        Self {
            id,
            response_code: DnsResponseCode::NxDomain,
            authoritative: false,
            recursion_available: true,
            answers: Vec::new(),
            authority: Vec::new(),
            additional: Vec::new(),
        }
    }

    /// Create a SERVFAIL response
    pub fn servfail(id: u16) -> Self {
        Self {
            id,
            response_code: DnsResponseCode::ServFail,
            authoritative: false,
            recursion_available: true,
            answers: Vec::new(),
            authority: Vec::new(),
            additional: Vec::new(),
        }
    }

    /// Create a REFUSED response
    pub fn refused(id: u16) -> Self {
        Self {
            id,
            response_code: DnsResponseCode::Refused,
            authoritative: false,
            recursion_available: true,
            answers: Vec::new(),
            authority: Vec::new(),
            additional: Vec::new(),
        }
    }

    /// Add an answer record
    pub fn add_answer(&mut self, record: DnsRecordData) {
        self.answers.push(record);
    }

    /// Smallest TTL across the answer records, if there are any.
    ///
    /// The whole answer set becomes stale as soon as its shortest-lived record
    /// does, so the minimum is what a cache may honour. `None` means there are
    /// no answers (an empty or negative response), leaving the caller to decide
    /// how long that fact holds.
    pub fn min_answer_ttl(&self) -> Option<u32> {
        self.answers.iter().map(|answer| answer.ttl).min()
    }

    /// Parse a DNS response from raw bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, DnsError> {
        let message = Message::from_bytes(data).map_err(|e| DnsError::ParseError(e.to_string()))?;

        let response_code = DnsResponseCode::from_trust_dns(message.response_code());

        let answers = message
            .answers()
            .iter()
            .filter_map(|r| record_to_data(r))
            .collect();

        let authority = message
            .name_servers()
            .iter()
            .filter_map(|r| record_to_data(r))
            .collect();

        let additional = message
            .additionals()
            .iter()
            .filter_map(|r| record_to_data(r))
            .collect();

        Ok(Self {
            id: message.id(),
            response_code,
            authoritative: message.authoritative(),
            recursion_available: message.recursion_available(),
            answers,
            authority,
            additional,
        })
    }

    /// Encode the DNS response to raw bytes for a given query
    pub fn to_bytes(&self, query: &DnsQuery) -> Result<Vec<u8>, DnsError> {
        self.to_message(query)?
            .to_bytes()
            .map_err(|e| DnsError::EncodeError(e.to_string()))
    }

    /// Encode the response for a datagram transport that cannot fragment.
    ///
    /// UDP has no way to signal "answer continues", so RFC 1035 section 4.2.1
    /// requires oversized answers to be sent with the TC bit set and the
    /// records dropped, letting the client retry over TCP. Emitting an
    /// oversized datagram instead would either be refused by the socket or be
    /// silently cut by the network, which is how truncated answers used to
    /// reach clients as if they were complete.
    pub fn to_bytes_for_udp(&self, query: &DnsQuery) -> Result<Vec<u8>, DnsError> {
        let full = self.to_bytes(query)?;
        let limit = query.max_udp_response_size();

        if full.len() <= limit {
            return Ok(full);
        }

        let mut header_only = Self {
            id: self.id,
            response_code: self.response_code,
            authoritative: self.authoritative,
            recursion_available: self.recursion_available,
            answers: Vec::new(),
            authority: Vec::new(),
            additional: Vec::new(),
        }
        .to_message(query)?;
        header_only.set_truncated(true);

        header_only
            .to_bytes()
            .map_err(|e| DnsError::EncodeError(e.to_string()))
    }

    /// Build the hickory message for this response without serializing it.
    fn to_message(&self, query: &DnsQuery) -> Result<Message, DnsError> {
        let query_name =
            Name::from_str(&query.name).map_err(|e| DnsError::InvalidDomainName(e.to_string()))?;

        let mut message = Message::new();
        message.set_id(self.id);
        message.set_message_type(MessageType::Response);
        message.set_op_code(OpCode::Query);
        message.set_authoritative(self.authoritative);
        message.set_recursion_desired(query.recursion_desired);
        message.set_recursion_available(self.recursion_available);
        message.set_response_code(self.response_code.to_trust_dns());

        message.add_query(hickory_proto::op::Query::query(
            query_name,
            query.record_type.to_trust_dns(),
        ));

        for answer in &self.answers {
            if let Some(record) = data_to_record(answer) {
                message.add_answer(record);
            }
        }

        for auth in &self.authority {
            if let Some(record) = data_to_record(auth) {
                message.add_name_server(record);
            }
        }

        for add in &self.additional {
            if let Some(record) = data_to_record(add) {
                message.add_additional(record);
            }
        }

        // Echo the client's EDNS(0) buffer size so it knows we honoured it.
        if let Some(payload_size) = query.udp_payload_size {
            let mut edns = hickory_proto::op::Edns::new();
            edns.set_max_payload(payload_size);
            message.set_edns(edns);
        }

        Ok(message)
    }
}

/// Convert a hickory-proto Record to DnsRecordData
fn record_to_data(record: &Record) -> Option<DnsRecordData> {
    let name = record.name().to_string().trim_end_matches('.').to_string();
    let ttl = record.ttl();

    match record.data() {
        RData::A(ip) => Some(DnsRecordData {
            name,
            record_type: RecordType::A,
            value: ip.to_string(),
            ttl,
            priority: None,
        }),
        RData::AAAA(ip) => Some(DnsRecordData {
            name,
            record_type: RecordType::AAAA,
            value: ip.to_string(),
            ttl,
            priority: None,
        }),
        RData::CNAME(cname) => Some(DnsRecordData {
            name,
            record_type: RecordType::CNAME,
            value: cname.to_string().trim_end_matches('.').to_string(),
            ttl,
            priority: None,
        }),
        RData::MX(mx) => Some(DnsRecordData {
            name,
            record_type: RecordType::MX,
            value: mx.exchange().to_string().trim_end_matches('.').to_string(),
            ttl,
            priority: Some(mx.preference()),
        }),
        RData::TXT(txt) => {
            let text: String = txt
                .txt_data()
                .iter()
                .map(|b| String::from_utf8_lossy(b).to_string())
                .collect::<Vec<_>>()
                .join("");
            Some(DnsRecordData {
                name,
                record_type: RecordType::TXT,
                value: text,
                ttl,
                priority: None,
            })
        }
        RData::PTR(ptr) => Some(DnsRecordData {
            name,
            record_type: RecordType::PTR,
            value: ptr.to_string().trim_end_matches('.').to_string(),
            ttl,
            priority: None,
        }),
        RData::NS(ns) => Some(DnsRecordData {
            name,
            record_type: RecordType::NS,
            value: ns.to_string().trim_end_matches('.').to_string(),
            ttl,
            priority: None,
        }),
        RData::SOA(soa) => {
            let value = format!(
                "{} {} {} {} {} {} {}",
                soa.mname().to_string().trim_end_matches('.'),
                soa.rname().to_string().trim_end_matches('.'),
                soa.serial(),
                soa.refresh(),
                soa.retry(),
                soa.expire(),
                soa.minimum()
            );
            Some(DnsRecordData {
                name,
                record_type: RecordType::SOA,
                value,
                ttl,
                priority: None,
            })
        }
        RData::SRV(srv) => {
            let value = format!(
                "{} {} {}",
                srv.weight(),
                srv.port(),
                srv.target().to_string().trim_end_matches('.')
            );
            Some(DnsRecordData {
                name,
                record_type: RecordType::SRV,
                value,
                ttl,
                priority: Some(srv.priority()),
            })
        }
        _ => None,
    }
}

/// Convert DnsRecordData to a hickory-proto Record
fn data_to_record(data: &DnsRecordData) -> Option<Record> {
    let name = Name::from_str(&data.name).ok()?;

    let rdata = match data.record_type {
        RecordType::A => {
            let ip: Ipv4Addr = data.value.parse().ok()?;
            RData::A(ip.into())
        }
        RecordType::AAAA => {
            let ip: Ipv6Addr = data.value.parse().ok()?;
            RData::AAAA(ip.into())
        }
        RecordType::CNAME => {
            let target = Name::from_str(&data.value).ok()?;
            RData::CNAME(hickory_proto::rr::rdata::CNAME(target))
        }
        RecordType::MX => {
            let exchange = Name::from_str(&data.value).ok()?;
            let priority = data.priority.unwrap_or(10);
            RData::MX(hickory_proto::rr::rdata::MX::new(priority, exchange))
        }
        RecordType::TXT => RData::TXT(hickory_proto::rr::rdata::TXT::new(vec![data.value.clone()])),
        RecordType::PTR => {
            let target = Name::from_str(&data.value).ok()?;
            RData::PTR(hickory_proto::rr::rdata::PTR(target))
        }
        RecordType::NS => {
            let ns = Name::from_str(&data.value).ok()?;
            RData::NS(hickory_proto::rr::rdata::NS(ns))
        }
        RecordType::SOA => {
            // Parse SOA format: "mname rname serial refresh retry expire minimum"
            let parts: Vec<&str> = data.value.split_whitespace().collect();
            if parts.len() != 7 {
                return None;
            }
            let mname = Name::from_str(parts[0]).ok()?;
            let rname = Name::from_str(parts[1]).ok()?;
            let serial: u32 = parts[2].parse().ok()?;
            let refresh: i32 = parts[3].parse().ok()?;
            let retry: i32 = parts[4].parse().ok()?;
            let expire: i32 = parts[5].parse().ok()?;
            let minimum: u32 = parts[6].parse().ok()?;
            RData::SOA(hickory_proto::rr::rdata::SOA::new(
                mname, rname, serial, refresh, retry, expire, minimum,
            ))
        }
        RecordType::SRV => {
            // Parse SRV format: "weight port target"
            let parts: Vec<&str> = data.value.split_whitespace().collect();
            if parts.len() != 3 {
                return None;
            }
            let weight: u16 = parts[0].parse().ok()?;
            let port: u16 = parts[1].parse().ok()?;
            let target = Name::from_str(parts[2]).ok()?;
            let priority = data.priority.unwrap_or(0);
            RData::SRV(hickory_proto::rr::rdata::SRV::new(
                priority, weight, port, target,
            ))
        }
    };

    Some(Record::from_rdata(name, data.ttl, rdata))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_type_from_str() {
        assert_eq!(RecordType::from_str("A").unwrap(), RecordType::A);
        assert_eq!(RecordType::from_str("aaaa").unwrap(), RecordType::AAAA);
        assert_eq!(RecordType::from_str("CNAME").unwrap(), RecordType::CNAME);
        assert_eq!(RecordType::from_str("mx").unwrap(), RecordType::MX);
        assert!(RecordType::from_str("INVALID").is_err());
    }

    #[test]
    fn test_record_type_display() {
        assert_eq!(RecordType::A.to_string(), "A");
        assert_eq!(RecordType::AAAA.to_string(), "AAAA");
        assert_eq!(RecordType::CNAME.to_string(), "CNAME");
    }

    #[test]
    fn test_dns_query_creation() {
        let query = DnsQuery::new("example.com", RecordType::A);
        assert_eq!(query.name, "example.com");
        assert_eq!(query.record_type, RecordType::A);
        assert!(query.recursion_desired);
    }

    #[test]
    fn test_dns_query_roundtrip() {
        let query = DnsQuery::with_id(12345, "example.com", RecordType::A);
        let bytes = query.to_bytes().unwrap();
        let parsed = DnsQuery::from_bytes(&bytes).unwrap();

        assert_eq!(parsed.id, 12345);
        assert_eq!(parsed.name, "example.com");
        assert_eq!(parsed.record_type, RecordType::A);
    }

    #[test]
    fn test_dns_response_creation() {
        let mut response = DnsResponse::new(12345);
        response.add_answer(DnsRecordData::a(
            "example.com",
            "93.184.216.34".parse().unwrap(),
            300,
        ));

        assert_eq!(response.id, 12345);
        assert_eq!(response.response_code, DnsResponseCode::NoError);
        assert_eq!(response.answers.len(), 1);
    }

    #[test]
    fn test_dns_response_nxdomain() {
        let response = DnsResponse::nxdomain(12345);
        assert_eq!(response.response_code, DnsResponseCode::NxDomain);
        assert!(response.answers.is_empty());
    }

    #[test]
    fn test_dns_record_data_a() {
        let record = DnsRecordData::a("example.com", "93.184.216.34".parse().unwrap(), 300);
        assert_eq!(record.name, "example.com");
        assert_eq!(record.record_type, RecordType::A);
        assert_eq!(record.value, "93.184.216.34");
        assert_eq!(record.ttl, 300);
    }

    #[test]
    fn test_dns_record_data_mx() {
        let record = DnsRecordData::mx("example.com", "mail.example.com", 10, 300);
        assert_eq!(record.record_type, RecordType::MX);
        assert_eq!(record.priority, Some(10));
    }

    #[test]
    fn test_response_code_display() {
        assert_eq!(DnsResponseCode::NoError.to_string(), "NOERROR");
        assert_eq!(DnsResponseCode::NxDomain.to_string(), "NXDOMAIN");
        assert_eq!(DnsResponseCode::ServFail.to_string(), "SERVFAIL");
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    // Feature: dns-proxy-service, Property 2: DNS 记录类型查询正确性
    // For any valid DNS query, encoding then decoding should produce an equivalent query

    /// Strategy to generate valid domain names
    fn domain_strategy() -> impl Strategy<Value = String> {
        // Generate valid domain labels (alphanumeric, 1-10 chars each)
        let label = "[a-z][a-z0-9]{0,9}";
        // Domain with 2-3 labels
        (label, label).prop_map(|(l1, l2)| format!("{}.{}", l1, l2))
    }

    /// Strategy to generate record types
    fn record_type_strategy() -> impl Strategy<Value = RecordType> {
        prop_oneof![
            Just(RecordType::A),
            Just(RecordType::AAAA),
            Just(RecordType::CNAME),
            Just(RecordType::MX),
            Just(RecordType::TXT),
            Just(RecordType::PTR),
            Just(RecordType::NS),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 2: DNS Query Round-Trip
        /// For any valid DNS query, encoding to bytes and decoding back should preserve
        /// the query ID, domain name, and record type.
        /// **Validates: Requirements 2.1-2.9**
        #[test]
        fn prop_dns_query_roundtrip(
            id in any::<u16>(),
            domain in domain_strategy(),
            record_type in record_type_strategy()
        ) {
            let query = DnsQuery::with_id(id, &domain, record_type);

            // Encode to bytes
            let bytes = query.to_bytes();
            prop_assert!(bytes.is_ok(), "Query encoding should succeed");

            // Decode from bytes
            let parsed = DnsQuery::from_bytes(&bytes.unwrap());
            prop_assert!(parsed.is_ok(), "Query decoding should succeed");

            let parsed = parsed.unwrap();

            // Verify round-trip preserves data
            prop_assert_eq!(parsed.id, id, "Query ID should be preserved");
            prop_assert_eq!(parsed.name.to_lowercase(), domain.to_lowercase(), "Domain name should be preserved");
            prop_assert_eq!(parsed.record_type, record_type, "Record type should be preserved");
        }

        /// Property 2: Record Type String Round-Trip
        /// For any record type, converting to string and back should produce the same type.
        /// **Validates: Requirements 2.1-2.9**
        #[test]
        fn prop_record_type_string_roundtrip(record_type in record_type_strategy()) {
            let type_str = record_type.to_string();
            let parsed = RecordType::from_str(&type_str);

            prop_assert!(parsed.is_ok(), "Record type string parsing should succeed");
            prop_assert_eq!(parsed.unwrap(), record_type, "Record type should be preserved after string round-trip");
        }

        /// Property 2: A Record Data Consistency
        /// For any valid IPv4 address and TTL, creating an A record should preserve all values.
        /// **Validates: Requirements 2.1**
        #[test]
        fn prop_a_record_consistency(
            domain in domain_strategy(),
            ip_octets in (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>()),
            ttl in 1u32..86400u32
        ) {
            let ip = Ipv4Addr::new(ip_octets.0, ip_octets.1, ip_octets.2, ip_octets.3);
            let record = DnsRecordData::a(&domain, ip, ttl);

            prop_assert_eq!(record.name, domain, "Domain name should be preserved");
            prop_assert_eq!(record.record_type, RecordType::A, "Record type should be A");
            prop_assert_eq!(record.value, ip.to_string(), "IP address should be preserved");
            prop_assert_eq!(record.ttl, ttl, "TTL should be preserved");
            prop_assert!(record.priority.is_none(), "A records should not have priority");
        }

        /// Property 2: AAAA Record Data Consistency
        /// For any valid IPv6 address and TTL, creating an AAAA record should preserve all values.
        /// **Validates: Requirements 2.2**
        #[test]
        fn prop_aaaa_record_consistency(
            domain in domain_strategy(),
            ip_segments in (any::<u16>(), any::<u16>(), any::<u16>(), any::<u16>(),
                           any::<u16>(), any::<u16>(), any::<u16>(), any::<u16>()),
            ttl in 1u32..86400u32
        ) {
            let ip = Ipv6Addr::new(
                ip_segments.0, ip_segments.1, ip_segments.2, ip_segments.3,
                ip_segments.4, ip_segments.5, ip_segments.6, ip_segments.7
            );
            let record = DnsRecordData::aaaa(&domain, ip, ttl);

            prop_assert_eq!(record.name, domain, "Domain name should be preserved");
            prop_assert_eq!(record.record_type, RecordType::AAAA, "Record type should be AAAA");
            prop_assert_eq!(record.ttl, ttl, "TTL should be preserved");
            prop_assert!(record.priority.is_none(), "AAAA records should not have priority");
        }

        /// Property 2: MX Record Priority Preservation
        /// For any MX record, the priority value should be preserved.
        /// **Validates: Requirements 2.4**
        #[test]
        fn prop_mx_record_priority(
            domain in domain_strategy(),
            exchange in domain_strategy(),
            priority in any::<u16>(),
            ttl in 1u32..86400u32
        ) {
            let record = DnsRecordData::mx(&domain, &exchange, priority, ttl);

            prop_assert_eq!(record.record_type, RecordType::MX, "Record type should be MX");
            prop_assert_eq!(record.priority, Some(priority), "Priority should be preserved");
            prop_assert_eq!(record.value, exchange, "Exchange should be preserved");
        }

        /// Property 2: DNS Response Code Consistency
        /// For any response code, converting to trust-dns and back should preserve the code.
        #[test]
        fn prop_response_code_roundtrip(code_value in 0u16..6u16) {
            let code = match code_value {
                0 => DnsResponseCode::NoError,
                1 => DnsResponseCode::FormErr,
                2 => DnsResponseCode::ServFail,
                3 => DnsResponseCode::NxDomain,
                4 => DnsResponseCode::NotImp,
                _ => DnsResponseCode::Refused,
            };

            let trust_code = code.to_trust_dns();
            let back = DnsResponseCode::from_trust_dns(trust_code);

            prop_assert_eq!(back, code, "Response code should be preserved after round-trip");
        }
    }
}
