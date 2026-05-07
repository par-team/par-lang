use num_bigint::{BigInt, BigUint, Sign};
use num_traits::ToPrimitive;

use jiff::civil::{DateTime, Weekday};
use jiff::tz::{Offset, TimeZone};
use jiff::{SignedDuration, Span, Timestamp, Zoned};

use par_runtime::atom::{Atom, sym};
use par_runtime::primitive::ParString;
use par_runtime::readback::Handle;
use par_runtime::registry::{DefinitionRef, ExternalDef, PackageRef};

macro_rules! core_time_external {
    ($name:literal, $f:path $(, $arg:expr)*) => {
        inventory::submit!(ExternalDef {
            path: DefinitionRef {
                package: PackageRef::CORE,
                path: &[],
                module: "Time",
                name: $name,
            },
            f: |handle| Box::pin($f(handle $(, $arg)*)),
        });
    };
}

core_time_external!("Now_", time_now);
core_time_external!("FromUnixNanos_", time_from_unix_nanos);
core_time_external!("Show", time_show);
core_time_external!("FromRFC3339", time_from_rfc3339);
core_time_external!("ToRFC3339", time_to_rfc3339);
core_time_external!("UTC", time_utc);
core_time_external!("Local", time_local);
core_time_external!("Offset", time_offset);
core_time_external!("Zone", time_zone);
core_time_external!("InZone", time_in_zone);
core_time_external!("At", time_at);
core_time_external!("Parse", time_parse);

const NANOS_PER_SEC: i128 = 1_000_000_000;

// jiff's largest representable fixed offset is ±25:59:59.
const MAX_OFFSET_SECONDS: i32 = 93_599;

fn bigint_to_i128_sat(n: &BigInt) -> i128 {
    n.to_i128().unwrap_or(match n.sign() {
        Sign::Minus => i128::MIN,
        _ => i128::MAX,
    })
}

fn bigint_to_i64_sat(n: &BigInt) -> i64 {
    n.to_i64().unwrap_or(match n.sign() {
        Sign::Minus => i64::MIN,
        _ => i64::MAX,
    })
}

fn timestamp_from_nanos(nanos: i128) -> Timestamp {
    Timestamp::from_nanosecond(nanos).unwrap_or(if nanos < 0 {
        Timestamp::MIN
    } else {
        Timestamp::MAX
    })
}

fn timestamp_from_bigint(nanos: &BigInt) -> Timestamp {
    timestamp_from_nanos(bigint_to_i128_sat(nanos))
}

fn signed_duration_from_nanos(nanos: i128) -> SignedDuration {
    let secs = (nanos / NANOS_PER_SEC).clamp(i64::MIN as i128, i64::MAX as i128) as i64;
    let sub = (nanos % NANOS_PER_SEC) as i32;
    SignedDuration::new(secs, sub)
}

fn timestamp_shift(timestamp: Timestamp, nanos: i128) -> Timestamp {
    timestamp
        .checked_add(signed_duration_from_nanos(nanos))
        .unwrap_or(if nanos < 0 {
            Timestamp::MIN
        } else {
            Timestamp::MAX
        })
}

fn offset_from_nanos(nanos: &BigInt) -> Offset {
    let secs = (nanos / BigInt::from(NANOS_PER_SEC))
        .to_i32()
        .unwrap_or(match nanos.sign() {
            Sign::Minus => -MAX_OFFSET_SECONDS,
            _ => MAX_OFFSET_SECONDS,
        })
        .clamp(-MAX_OFFSET_SECONDS, MAX_OFFSET_SECONDS);
    Offset::from_seconds(secs).unwrap_or(Offset::UTC)
}

async fn read_instant_nanos(mut instant: Handle) -> i128 {
    instant.signal(sym::unixNanos);
    bigint_to_i128_sat(&instant.int().await)
}

fn provide_instant(handle: Handle, timestamp: Timestamp) {
    handle.provide_box(move |mut handle| {
        let timestamp = timestamp;
        async move {
            loop {
                match handle.case().await {
                    sym::unixNanos => {
                        handle.provide_int(BigInt::from(timestamp.as_nanosecond()));
                        return;
                    }
                    sym::add => {
                        let duration = bigint_to_i128_sat(&handle.receive().int().await);
                        return provide_instant(handle, timestamp_shift(timestamp, duration));
                    }
                    _ => unreachable!(),
                }
            }
        }
    });
}

fn zone_name(tz: &TimeZone) -> String {
    if let Some(name) = tz.iana_name() {
        return name.to_string();
    }
    tz.to_offset(Timestamp::UNIX_EPOCH).to_string()
}

fn provide_zone(handle: Handle, tz: TimeZone) {
    handle.provide_box(move |mut handle| {
        let tz = tz.clone();
        async move {
            match handle.case().await {
                sym::name => {
                    handle.provide_string(ParString::from(zone_name(&tz)));
                }
                sym::offsetAt => {
                    let nanos = read_instant_nanos(handle.receive()).await;
                    let offset = tz.to_offset(timestamp_from_nanos(nanos));
                    let offset_nanos = (offset.seconds() as i128) * NANOS_PER_SEC;
                    handle.provide_int(BigInt::from(offset_nanos));
                }
                _ => unreachable!(),
            }
        }
    });
}

// Total for any structural Zone value: a known IANA name resolves to its full
// DST-aware zone; anything else falls back to the fixed offset at `reference`.
async fn zone_to_timezone(mut zone: Handle, reference: Timestamp) -> TimeZone {
    let mut zone_offset = zone.duplicate();
    zone.signal(sym::name);
    let name = zone.string().await;
    if let Ok(tz) = TimeZone::get(name.as_str()) {
        zone_offset.erase();
        tz
    } else {
        zone_offset.signal(sym::offsetAt);
        provide_instant(zone_offset.send(), reference);
        let offset_nanos = zone_offset.int().await;
        TimeZone::fixed(offset_from_nanos(&offset_nanos))
    }
}

fn weekday_label(weekday: Weekday) -> Atom {
    match weekday {
        Weekday::Monday => sym::monday,
        Weekday::Tuesday => sym::tuesday,
        Weekday::Wednesday => sym::wednesday,
        Weekday::Thursday => sym::thursday,
        Weekday::Friday => sym::friday,
        Weekday::Saturday => sym::saturday,
        Weekday::Sunday => sym::sunday,
    }
}

fn add_calendar(zoned: &Zoned, amount: &BigInt, unit: char) -> Zoned {
    let amount = bigint_to_i64_sat(amount);
    // Clamp to a span size that can't overflow; the add below then saturates.
    let span = match unit {
        'y' => Span::new().years(amount.clamp(-19_998, 19_998)),
        'm' => Span::new().months(amount.clamp(-239_976, 239_976)),
        _ => Span::new().days(amount.clamp(-7_304_484, 7_304_484)),
    };
    zoned.checked_add(span).unwrap_or_else(|_| zoned.clone())
}

fn provide_zoned(handle: Handle, zoned: Zoned) {
    handle.provide_box(move |mut handle| {
        let zoned = zoned.clone();
        async move {
            loop {
                match handle.case().await {
                    sym::year => {
                        handle.provide_int(BigInt::from(zoned.year()));
                        return;
                    }
                    sym::month => {
                        handle.provide_nat(BigUint::from(zoned.month() as u8));
                        return;
                    }
                    sym::day => {
                        handle.provide_nat(BigUint::from(zoned.day() as u8));
                        return;
                    }
                    sym::hour => {
                        handle.provide_nat(BigUint::from(zoned.hour() as u8));
                        return;
                    }
                    sym::minute => {
                        handle.provide_nat(BigUint::from(zoned.minute() as u8));
                        return;
                    }
                    sym::second => {
                        handle.provide_nat(BigUint::from(zoned.second() as u8));
                        return;
                    }
                    sym::nanosecond => {
                        handle.provide_nat(BigUint::from(zoned.subsec_nanosecond() as u32));
                        return;
                    }
                    sym::weekday => {
                        handle.signal(weekday_label(zoned.weekday()));
                        handle.break_();
                        return;
                    }
                    sym::zone => {
                        return provide_zone(handle, zoned.time_zone().clone());
                    }
                    sym::instant => {
                        return provide_instant(handle, zoned.timestamp());
                    }
                    sym::format => {
                        let layout = handle.receive().string().await;
                        let rendered = zoned.strftime(layout.as_str()).to_string();
                        handle.provide_string(ParString::from(rendered));
                        return;
                    }
                    sym::addYears => {
                        let amount = handle.receive().int().await;
                        return provide_zoned(handle, add_calendar(&zoned, &amount, 'y'));
                    }
                    sym::addMonths => {
                        let amount = handle.receive().int().await;
                        return provide_zoned(handle, add_calendar(&zoned, &amount, 'm'));
                    }
                    sym::addDays => {
                        let amount = handle.receive().int().await;
                        return provide_zoned(handle, add_calendar(&zoned, &amount, 'd'));
                    }
                    _ => unreachable!(),
                }
            }
        }
    });
}

fn format_duration(nanos: &BigInt) -> String {
    if nanos.sign() == Sign::NoSign {
        return "0s".to_string();
    }
    let negative = nanos.sign() == Sign::Minus;
    let mut rem = if negative { -nanos } else { nanos.clone() };

    let units: [(&str, i128); 6] = [
        ("h", 3_600 * NANOS_PER_SEC),
        ("m", 60 * NANOS_PER_SEC),
        ("s", NANOS_PER_SEC),
        ("ms", 1_000_000),
        ("us", 1_000),
        ("ns", 1),
    ];

    let mut out = String::new();
    if negative {
        out.push('-');
    }
    for (suffix, size) in units {
        let size = BigInt::from(size);
        let quotient = &rem / &size;
        if quotient.sign() != Sign::NoSign {
            out.push_str(&quotient.to_string());
            out.push_str(suffix);
            rem %= &size;
        }
    }
    out
}

// None for any out-of-range field, and hence for invalid dates such as Feb 30.
fn civil_from_fields(
    year: &BigInt,
    month: &BigUint,
    day: &BigUint,
    hour: &BigUint,
    minute: &BigUint,
    second: &BigUint,
) -> Option<DateTime> {
    let year = year.to_i16()?;
    let month = month.to_i8()?;
    let day = day.to_i8()?;
    let hour = hour.to_i8()?;
    let minute = minute.to_i8()?;
    let second = second.to_i8()?;
    DateTime::new(year, month, day, hour, minute, second, 0).ok()
}

async fn zoned_from_civil(mut handle: Handle, zone: Handle, datetime: DateTime) {
    let reference = datetime
        .to_zoned(TimeZone::UTC)
        .map(|z| z.timestamp())
        .unwrap_or(Timestamp::UNIX_EPOCH);
    let tz = zone_to_timezone(zone, reference).await;
    match datetime.to_zoned(tz) {
        Ok(zoned) => {
            handle.signal(sym::some);
            provide_zoned(handle, zoned);
        }
        Err(_) => provide_none(handle),
    }
}

fn provide_none(mut handle: Handle) {
    handle.signal(sym::none);
    handle.break_();
}

async fn time_now(mut handle: Handle) {
    handle.receive().continue_();
    let now = Timestamp::now();
    handle.provide_int(BigInt::from(now.as_nanosecond()));
}

async fn time_from_unix_nanos(mut handle: Handle) {
    let nanos = handle.receive().int().await;
    provide_instant(handle, timestamp_from_bigint(&nanos));
}

async fn time_show(mut handle: Handle) {
    let nanos = handle.receive().int().await;
    handle.provide_string(ParString::from(format_duration(&nanos)));
}

async fn time_from_rfc3339(mut handle: Handle) {
    let text = handle.receive().string().await;
    match text.as_str().parse::<Timestamp>() {
        Ok(timestamp) => {
            handle.signal(sym::some);
            provide_instant(handle, timestamp);
        }
        Err(_) => provide_none(handle),
    }
}

async fn time_to_rfc3339(mut handle: Handle) {
    let nanos = read_instant_nanos(handle.receive()).await;
    let timestamp = timestamp_from_nanos(nanos);
    handle.provide_string(ParString::from(timestamp.to_string()));
}

async fn time_utc(handle: Handle) {
    provide_zone(handle, TimeZone::UTC);
}

async fn time_local(handle: Handle) {
    provide_zone(handle, TimeZone::system());
}

async fn time_offset(mut handle: Handle) {
    let nanos = handle.receive().int().await;
    provide_zone(handle, TimeZone::fixed(offset_from_nanos(&nanos)));
}

async fn time_zone(mut handle: Handle) {
    let name = handle.receive().string().await;
    match TimeZone::get(name.as_str()) {
        Ok(tz) => {
            handle.signal(sym::some);
            provide_zone(handle, tz);
        }
        Err(_) => provide_none(handle),
    }
}

async fn time_in_zone(mut handle: Handle) {
    let nanos = read_instant_nanos(handle.receive()).await;
    let timestamp = timestamp_from_nanos(nanos);
    let tz = zone_to_timezone(handle.receive(), timestamp).await;
    provide_zoned(handle, timestamp.to_zoned(tz));
}

async fn time_at(mut handle: Handle) {
    let zone = handle.receive();
    let year = handle.receive().int().await;
    let month = handle.receive().nat().await;
    let day = handle.receive().nat().await;
    let hour = handle.receive().nat().await;
    let minute = handle.receive().nat().await;
    let second = handle.receive().nat().await;
    match civil_from_fields(&year, &month, &day, &hour, &minute, &second) {
        Some(datetime) => zoned_from_civil(handle, zone, datetime).await,
        None => {
            zone.erase();
            provide_none(handle);
        }
    }
}

async fn time_parse(mut handle: Handle) {
    let zone = handle.receive();
    let layout = handle.receive().string().await;
    let text = handle.receive().string().await;
    match DateTime::strptime(layout.as_str(), text.as_str()) {
        Ok(datetime) => zoned_from_civil(handle, zone, datetime).await,
        Err(_) => {
            zone.erase();
            provide_none(handle);
        }
    }
}
