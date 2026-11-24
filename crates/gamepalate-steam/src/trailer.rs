use std::time::Duration;

use reqwest::{
    Client, Method, Request, StatusCode, Url,
    header::{ACCEPT, HeaderValue},
};
use roxmltree::{Document, Node};
use tokio::time::timeout;

use crate::transport::{Fetch, USER_AGENT};

const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_HEIGHT: u32 = 1080;
const AV1: &str = "av01";
const AAC: &str = "mp4a.40";
const REPRESENTATION_ID: &str = "$RepresentationID$";
const NUMBER: &str = "$Number";
const MANIFEST_ACCEPT: &str = "application/dash+xml";
const SEGMENT_ACCEPT: &str = "video/mp4, audio/mp4";

#[derive(Debug, thiserror::Error)]
pub enum TrailerError {
    #[error("{url} is not a valid trailer address")]
    Url { url: String },

    #[error("could not download {url}: {reason}")]
    Network {
        url: String,
        reason: String,
        #[source]
        source: Option<reqwest::Error>,
    },

    #[error("{url} answered HTTP {status}")]
    Status { url: String, status: StatusCode },

    #[error("{url} is larger than {} MB", MAX_BYTES / 1024 / 1024)]
    TooLarge { url: String },

    #[error("the trailer manifest {url} is not valid XML")]
    Xml {
        url: String,
        #[source]
        source: roxmltree::Error,
    },

    #[error("the trailer manifest {url} has {reason}")]
    Manifest { url: String, reason: String },

    #[error("could not set up the HTTP client for trailers")]
    Client(#[source] reqwest::Error),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub duration: Duration,
    pub video: Vec<VideoRendition>,
    pub audio: AudioRendition,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoRendition {
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub bandwidth: u64,
    pub segments: Segments,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioRendition {
    pub id: String,
    pub bandwidth: u64,
    pub segments: Segments,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segments {
    pub timescale: u64,
    pub duration: u64,
    pub start_number: u32,
    pub count: u32,
    base: Url,
    init: Url,
    before_number: String,
    number_width: usize,
    after_number: String,
}

impl Manifest {
    pub fn parse(url: &str, xml: &str) -> Result<Manifest, TrailerError> {
        let base = Url::parse(url).map_err(|_| TrailerError::Url {
            url: url.to_string(),
        })?;
        let document = Document::parse(xml).map_err(|source| TrailerError::Xml {
            url: url.to_string(),
            source,
        })?;
        let invalid = |reason: String| TrailerError::Manifest {
            url: url.to_string(),
            reason,
        };

        let mpd = document.root_element();
        let duration = mpd
            .attribute("mediaPresentationDuration")
            .ok_or_else(|| invalid("no mediaPresentationDuration".to_string()))?;
        let duration = iso_duration(duration)
            .ok_or_else(|| invalid(format!("an unreadable duration {duration}")))?;

        let mut video = Vec::new();
        let mut audio = None;
        for representation in mpd
            .descendants()
            .filter(|node| node.has_tag_name("Representation"))
        {
            let codecs = representation.attribute("codecs").unwrap_or_default();
            if codecs.starts_with(AV1) {
                let segments = segments(&base, representation, duration).map_err(&invalid)?;
                video.push(VideoRendition {
                    id: id(representation).map_err(&invalid)?.to_string(),
                    width: number(representation, "width").map_err(&invalid)?,
                    height: number(representation, "height").map_err(&invalid)?,
                    bandwidth: number(representation, "bandwidth").map_err(&invalid)?,
                    segments,
                });
            } else if codecs.starts_with(AAC) && audio.is_none() {
                audio = Some(AudioRendition {
                    id: id(representation).map_err(&invalid)?.to_string(),
                    bandwidth: number(representation, "bandwidth").map_err(&invalid)?,
                    segments: segments(&base, representation, duration).map_err(&invalid)?,
                });
            }
        }

        if video.is_empty() {
            return Err(invalid("no AV1 video".to_string()));
        }
        let audio = audio.ok_or_else(|| invalid("no AAC audio".to_string()))?;

        Ok(Manifest {
            duration,
            video,
            audio,
        })
    }

    pub fn video_for_height(&self, height: u32) -> &VideoRendition {
        let mut allowed: Vec<&VideoRendition> = self
            .video
            .iter()
            .filter(|video| video.height <= MAX_HEIGHT)
            .collect();
        if allowed.is_empty() {
            return self
                .video
                .iter()
                .min_by_key(|video| video.height)
                .expect("a manifest has some video");
        }
        allowed.sort_by_key(|video| video.height);

        allowed
            .iter()
            .find(|video| video.height >= height)
            .or(allowed.last())
            .expect("some video is allowed")
    }
}

impl Segments {
    pub fn init_url(&self) -> &Url {
        &self.init
    }

    pub fn url(&self, number: u32) -> Url {
        self.base
            .join(&self.media(number))
            .expect("the template made a valid address when the manifest was read")
    }

    pub fn nominal_duration(&self) -> Duration {
        Duration::from_nanos(
            (u128::from(self.duration) * 1_000_000_000 / u128::from(self.timescale)) as u64,
        )
    }

    pub fn at(&self, time: Duration) -> u32 {
        let ticks = time.as_nanos() * u128::from(self.timescale) / 1_000_000_000;
        let index = ticks / u128::from(self.duration);
        let index = u32::try_from(index).unwrap_or(u32::MAX).min(self.count - 1);

        self.start_number + index
    }

    pub fn last(&self) -> u32 {
        self.start_number + self.count - 1
    }

    fn media(&self, number: u32) -> String {
        format!(
            "{}{number:0width$}{}",
            self.before_number,
            self.after_number,
            width = self.number_width
        )
    }
}

fn segments(base: &Url, representation: Node, total: Duration) -> Result<Segments, String> {
    let template = representation
        .children()
        .chain(
            representation
                .parent_element()
                .into_iter()
                .flat_map(|set| set.children()),
        )
        .find(|node| node.has_tag_name("SegmentTemplate"))
        .ok_or_else(|| "a representation without a SegmentTemplate".to_string())?;
    let id = id(representation)?;

    let timescale = match template.attribute("timescale") {
        Some(_) => number(template, "timescale")?,
        None => 1,
    };
    let duration: u64 = number(template, "duration")?;
    if timescale == 0 || duration == 0 {
        return Err("a SegmentTemplate with a zero duration".to_string());
    }
    let start_number = match template.attribute("startNumber") {
        Some(_) => number(template, "startNumber")?,
        None => 1,
    };

    let init = template
        .attribute("initialization")
        .ok_or_else(|| "a SegmentTemplate without initialization".to_string())?
        .replace(REPRESENTATION_ID, id);
    let init = base
        .join(&init)
        .map_err(|_| format!("an unusable initialization {init}"))?;

    let media = template
        .attribute("media")
        .ok_or_else(|| "a SegmentTemplate without media".to_string())?
        .replace(REPRESENTATION_ID, id);
    let (before_number, number_width, after_number) = number_template(&media)
        .ok_or_else(|| format!("a media template without $Number$: {media}"))?;

    let total_ticks = total.as_nanos() * u128::from(timescale) / 1_000_000_000;
    let count = total_ticks.div_ceil(u128::from(duration)).max(1);
    let count = u32::try_from(count).map_err(|_| "too many segments".to_string())?;

    let segments = Segments {
        timescale,
        duration,
        start_number,
        count,
        base: base.clone(),
        init,
        before_number,
        number_width,
        after_number,
    };
    base.join(&segments.media(start_number))
        .map_err(|_| format!("an unusable media template {media}"))?;

    Ok(segments)
}

fn number_template(media: &str) -> Option<(String, usize, String)> {
    let start = media.find(NUMBER)?;
    let rest = &media[start + NUMBER.len()..];
    let end = rest.find('$')?;
    let format = &rest[..end];
    let width = match format {
        "" => 1,
        _ => format.strip_prefix("%0")?.strip_suffix('d')?.parse().ok()?,
    };

    Some((
        media[..start].to_string(),
        width,
        rest[end + 1..].to_string(),
    ))
}

fn id<'a>(representation: Node<'a, '_>) -> Result<&'a str, String> {
    representation
        .attribute("id")
        .ok_or_else(|| "a representation without an id".to_string())
}

fn number<T: std::str::FromStr>(node: Node, name: &str) -> Result<T, String> {
    let value = node
        .attribute(name)
        .ok_or_else(|| format!("a {} without {name}", node.tag_name().name()))?;

    value
        .parse()
        .map_err(|_| format!("an unreadable {name} {value}"))
}

fn iso_duration(text: &str) -> Option<Duration> {
    let rest = text.strip_prefix('P')?;
    let (days, time) = match rest.split_once('T') {
        Some((days, time)) => (days, time),
        None => (rest, ""),
    };
    let mut seconds = match days {
        "" => 0.0,
        _ => days.strip_suffix('D')?.parse::<f64>().ok()? * 86_400.0,
    };

    let mut number = String::new();
    for character in time.chars() {
        let unit = match character {
            'H' => 3_600.0,
            'M' => 60.0,
            'S' => 1.0,
            _ => {
                number.push(character);
                continue;
            }
        };
        seconds += number.parse::<f64>().ok()? * unit;
        number.clear();
    }
    if !number.is_empty() || !seconds.is_finite() || seconds < 0.0 {
        return None;
    }

    Some(Duration::from_secs_f64(seconds))
}

#[derive(Debug)]
pub struct TrailerDownloader<F = Client> {
    fetch: F,
}

impl TrailerDownloader {
    pub fn new() -> Result<TrailerDownloader, TrailerError> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(TrailerError::Client)?;

        Ok(TrailerDownloader::with_fetch(client))
    }
}

impl<F: Fetch> TrailerDownloader<F> {
    pub fn with_fetch(fetch: F) -> TrailerDownloader<F> {
        TrailerDownloader { fetch }
    }

    pub async fn manifest(&self, url: &str) -> Result<Manifest, TrailerError> {
        let parsed = Url::parse(url).map_err(|_| TrailerError::Url {
            url: url.to_string(),
        })?;
        let bytes = self.download(parsed, MANIFEST_ACCEPT).await?;
        let xml = String::from_utf8_lossy(&bytes);

        Manifest::parse(url, &xml)
    }

    pub async fn segment(&self, url: &Url) -> Result<Vec<u8>, TrailerError> {
        self.download(url.clone(), SEGMENT_ACCEPT).await
    }

    async fn download(&self, url: Url, accept: &'static str) -> Result<Vec<u8>, TrailerError> {
        let text = url.to_string();
        match timeout(TIMEOUT, self.download_now(url, accept)).await {
            Ok(result) => result,
            Err(_) => Err(TrailerError::Network {
                url: text,
                reason: "timed out".to_string(),
                source: None,
            }),
        }
    }

    async fn download_now(&self, url: Url, accept: &'static str) -> Result<Vec<u8>, TrailerError> {
        let text = url.to_string();
        let mut request = Request::new(Method::GET, url);
        request
            .headers_mut()
            .insert(ACCEPT, HeaderValue::from_static(accept));

        let mut response = self
            .fetch
            .fetch(request)
            .await
            .map_err(|error| network(&text, error))?;

        let status = response.status();
        if !status.is_success() {
            return Err(TrailerError::Status { url: text, status });
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BYTES as u64)
        {
            return Err(TrailerError::TooLarge { url: text });
        }

        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| network(&text, error))?
        {
            if bytes.len() + chunk.len() > MAX_BYTES {
                return Err(TrailerError::TooLarge { url: text });
            }
            bytes.extend_from_slice(&chunk);
        }

        Ok(bytes)
    }
}

fn network(url: &str, error: reqwest::Error) -> TrailerError {
    TrailerError::Network {
        url: url.to_string(),
        reason: "no answer".to_string(),
        source: Some(error.without_url()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use reqwest::Response;

    use super::*;

    const HADES_URL: &str = "https://video.akamai.steamstatic.com/store_trailers/1145360/312686/b0312c49135823cc34946e98148abfb8da71f0fb/1750627549/dash_av1.mpd";
    const HADES_BASE: &str = "https://video.akamai.steamstatic.com/store_trailers/1145360/312686/b0312c49135823cc34946e98148abfb8da71f0fb/1750627549/";
    const PORTAL_2_URL: &str = "https://video.akamai.steamstatic.com/store_trailers/620/3746/0f4bee66d490b637707ac4fca3e850f2cc2d3b2a/1750108721/dash_av1.mpd";

    const HADES: &str = r#"<?xml version='1.0' encoding='utf-8'?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="urn:mpeg:DASH:schema:MPD:2011 http://standards.iso.org/ittf/PubliclyAvailableStandards/MPEG-DASH_schema_files/DASH-MPD.xsd" profiles="urn:mpeg:dash:profile:isoff-live:2011" type="static" mediaPresentationDuration="PT1M25.5S" maxSegmentDuration="PT3.0S" minBufferTime="PT5.9S">
	<ProgramInformation>
	</ProgramInformation>
	<ServiceDescription id="0">
	</ServiceDescription>
	<Period id="0" start="PT0.0S">
		<AdaptationSet id="0" contentType="video" startWithSAP="1" segmentAlignment="true" bitstreamSwitching="true" frameRate="60000/1001" maxWidth="1920" maxHeight="1080" par="16:9">
			<Representation id="0" mimeType="video/mp4" codecs="av01.0.09M.08.0.111.01.01.01.0" bandwidth="5800000" width="1920" height="1080" sar="1:1">
				<SegmentTemplate timescale="1000000" duration="3000000" initialization="dash_av1/init-stream$RepresentationID$.m4s" media="dash_av1/chunk-stream$RepresentationID$-$Number%05d$.m4s" startNumber="1">
				</SegmentTemplate>
			</Representation>
			<Representation id="1" mimeType="video/mp4" codecs="av01.0.08M.08.0.111.01.01.01.0" bandwidth="2600000" width="1280" height="720" sar="1:1">
				<SegmentTemplate timescale="1000000" duration="3000000" initialization="dash_av1/init-stream$RepresentationID$.m4s" media="dash_av1/chunk-stream$RepresentationID$-$Number%05d$.m4s" startNumber="1">
				</SegmentTemplate>
			</Representation>
			<Representation id="2" mimeType="video/mp4" codecs="av01.0.05M.08.0.111.01.01.01.0" bandwidth="1400000" width="854" height="480" sar="1280:1281">
				<SegmentTemplate timescale="1000000" duration="3000000" initialization="dash_av1/init-stream$RepresentationID$.m4s" media="dash_av1/chunk-stream$RepresentationID$-$Number%05d$.m4s" startNumber="1">
				</SegmentTemplate>
			</Representation>
			<Representation id="3" mimeType="video/mp4" codecs="av01.0.04M.08.0.111.01.01.01.0" bandwidth="1000000" width="640" height="360" sar="1:1">
				<SegmentTemplate timescale="1000000" duration="3000000" initialization="dash_av1/init-stream$RepresentationID$.m4s" media="dash_av1/chunk-stream$RepresentationID$-$Number%05d$.m4s" startNumber="1">
				</SegmentTemplate>
			</Representation>
		</AdaptationSet>
		<AdaptationSet id="1" contentType="audio" startWithSAP="1" segmentAlignment="true" bitstreamSwitching="true" lang="eng">
			<Representation id="4" mimeType="audio/mp4" codecs="mp4a.40.2" bandwidth="192000" audioSamplingRate="48000">
				<AudioChannelConfiguration schemeIdUri="urn:mpeg:dash:23003:3:audio_channel_configuration:2011" value="2" />
				<SegmentTemplate timescale="1000000" duration="3000000" initialization="dash_av1/init-stream$RepresentationID$.m4s" media="dash_av1/chunk-stream$RepresentationID$-$Number%05d$.m4s" startNumber="1">
				</SegmentTemplate>
			</Representation>
		</AdaptationSet>
		<AdaptationSet id="2" mimeType="image/avif" contentType="image">
			<SegmentTemplate media="$RepresentationID$/thumbnails-$Number%05d$.avif" duration="90" startNumber="1" />
			<Representation id="dash_thumbnails" bandwidth="0" width="1280" height="864">
				<EssentialProperty schemeIdUri="http://dashif.org/guidelines/thumbnail_tile" value="5x6" />
			</Representation>
		</AdaptationSet>
	</Period>
</MPD>"#;

    const PORTAL_2: &str = r#"<?xml version='1.0' encoding='utf-8'?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="urn:mpeg:DASH:schema:MPD:2011 http://standards.iso.org/ittf/PubliclyAvailableStandards/MPEG-DASH_schema_files/DASH-MPD.xsd" profiles="urn:mpeg:dash:profile:isoff-live:2011" type="static" mediaPresentationDuration="PT1M45.6S" maxSegmentDuration="PT3.0S" minBufferTime="PT6.0S">
	<Period id="0" start="PT0.0S">
		<AdaptationSet id="0" contentType="video" startWithSAP="1" segmentAlignment="true" bitstreamSwitching="true" frameRate="30000/1001" maxWidth="940" maxHeight="528" par="940:529">
			<Representation id="0" mimeType="video/mp4" codecs="av01.0.04M.08.0.110.02.02.05.0" bandwidth="2600000" width="940" height="528" sar="528:529">
				<SegmentTemplate timescale="1000000" duration="3000000" initialization="dash_av1/init-stream$RepresentationID$.m4s" media="dash_av1/chunk-stream$RepresentationID$-$Number%05d$.m4s" startNumber="1">
				</SegmentTemplate>
			</Representation>
			<Representation id="1" mimeType="video/mp4" codecs="av01.0.04M.08.0.110.02.02.05.0" bandwidth="1400000" width="854" height="480" sar="225600:225883">
				<SegmentTemplate timescale="1000000" duration="3000000" initialization="dash_av1/init-stream$RepresentationID$.m4s" media="dash_av1/chunk-stream$RepresentationID$-$Number%05d$.m4s" startNumber="1">
				</SegmentTemplate>
			</Representation>
			<Representation id="2" mimeType="video/mp4" codecs="av01.0.01M.08.0.110.02.02.05.0" bandwidth="1000000" width="640" height="360" sar="2115:2116">
				<SegmentTemplate timescale="1000000" duration="3000000" initialization="dash_av1/init-stream$RepresentationID$.m4s" media="dash_av1/chunk-stream$RepresentationID$-$Number%05d$.m4s" startNumber="1">
				</SegmentTemplate>
			</Representation>
		</AdaptationSet>
		<AdaptationSet id="1" contentType="audio" startWithSAP="1" segmentAlignment="true" bitstreamSwitching="true">
			<Representation id="3" mimeType="audio/mp4" codecs="mp4a.40.2" bandwidth="192000" audioSamplingRate="48000">
				<AudioChannelConfiguration schemeIdUri="urn:mpeg:dash:23003:3:audio_channel_configuration:2011" value="2" />
				<SegmentTemplate timescale="1000000" duration="3000000" initialization="dash_av1/init-stream$RepresentationID$.m4s" media="dash_av1/chunk-stream$RepresentationID$-$Number%05d$.m4s" startNumber="1">
				</SegmentTemplate>
			</Representation>
		</AdaptationSet>
	</Period>
</MPD>"#;

    fn hades() -> Manifest {
        Manifest::parse(HADES_URL, HADES).unwrap()
    }

    fn portal_2() -> Manifest {
        Manifest::parse(PORTAL_2_URL, PORTAL_2).unwrap()
    }

    fn sizes(manifest: &Manifest) -> Vec<(&str, u32, u32, u64)> {
        manifest
            .video
            .iter()
            .map(|video| {
                (
                    video.id.as_str(),
                    video.width,
                    video.height,
                    video.bandwidth,
                )
            })
            .collect()
    }

    #[test]
    fn reads_the_av1_renditions_and_the_audio_of_hades() {
        let manifest = hades();

        assert_eq!(
            sizes(&manifest),
            vec![
                ("0", 1920, 1080, 5_800_000),
                ("1", 1280, 720, 2_600_000),
                ("2", 854, 480, 1_400_000),
                ("3", 640, 360, 1_000_000),
            ]
        );
        assert_eq!(manifest.audio.id, "4");
        assert_eq!(manifest.audio.bandwidth, 192_000);
    }

    #[test]
    fn reads_the_older_renditions_of_portal_2() {
        let manifest = portal_2();

        assert_eq!(
            sizes(&manifest),
            vec![
                ("0", 940, 528, 2_600_000),
                ("1", 854, 480, 1_400_000),
                ("2", 640, 360, 1_000_000),
            ]
        );
        assert_eq!(manifest.audio.id, "3");
        assert_eq!(manifest.duration, Duration::from_millis(105_600));
    }

    #[test]
    fn reads_the_segment_template_and_the_total_duration() {
        let manifest = hades();
        let segments = &manifest.video[0].segments;

        assert_eq!(manifest.duration, Duration::from_millis(85_500));
        assert_eq!(segments.timescale, 1_000_000);
        assert_eq!(segments.duration, 3_000_000);
        assert_eq!(segments.start_number, 1);
        assert_eq!(segments.count, 29);
        assert_eq!(segments.last(), 29);
        assert_eq!(segments.nominal_duration(), Duration::from_secs(3));
    }

    #[test]
    fn resolves_the_addresses_against_the_manifest() {
        let manifest = hades();
        let video = &manifest.video[1].segments;
        let audio = &manifest.audio.segments;

        assert_eq!(
            video.init_url().as_str(),
            format!("{HADES_BASE}dash_av1/init-stream1.m4s")
        );
        assert_eq!(
            video.url(1).as_str(),
            format!("{HADES_BASE}dash_av1/chunk-stream1-00001.m4s")
        );
        assert_eq!(
            audio.url(29).as_str(),
            format!("{HADES_BASE}dash_av1/chunk-stream4-00029.m4s")
        );
        assert_eq!(
            audio.init_url().as_str(),
            format!("{HADES_BASE}dash_av1/init-stream4.m4s")
        );
    }

    #[test]
    fn finds_the_segment_of_an_instant() {
        let segments = &hades().video[0].segments;

        assert_eq!(segments.at(Duration::ZERO), 1);
        assert_eq!(segments.at(Duration::from_millis(2_999)), 1);
        assert_eq!(segments.at(Duration::from_secs(3)), 2);
        assert_eq!(segments.at(Duration::from_secs(84)), 29);
        assert_eq!(segments.at(Duration::from_secs(600)), 29);
    }

    #[test]
    fn picks_the_smallest_rendition_tall_enough() {
        let hades = hades();
        let portal_2 = portal_2();
        let height = |manifest: &Manifest, asked| manifest.video_for_height(asked).height;

        assert_eq!(height(&hades, 400), 480);
        assert_eq!(height(&hades, 720), 720);
        assert_eq!(height(&hades, 1440), 1080);
        assert_eq!(height(&hades, 360), 360);
        assert_eq!(height(&portal_2, 400), 480);
        assert_eq!(height(&portal_2, 720), 528);
        assert_eq!(height(&portal_2, 1440), 528);
        assert_eq!(height(&portal_2, 360), 360);
    }

    #[test]
    fn never_picks_more_than_1080p() {
        let xml = HADES.replace(
            r#"width="1920" height="1080""#,
            r#"width="3840" height="2160""#,
        );
        let manifest = Manifest::parse(HADES_URL, &xml).unwrap();

        assert_eq!(manifest.video_for_height(1440).height, 720);
    }

    #[test]
    fn a_manifest_without_av1_is_an_error() {
        let xml = HADES.replace("codecs=\"av01", "codecs=\"avc1");

        assert!(matches!(
            Manifest::parse(HADES_URL, &xml),
            Err(TrailerError::Manifest { reason, .. }) if reason == "no AV1 video"
        ));
    }

    #[test]
    fn a_manifest_without_aac_is_an_error() {
        let xml = HADES.replace("mp4a.40.2", "opus");

        assert!(matches!(
            Manifest::parse(HADES_URL, &xml),
            Err(TrailerError::Manifest { reason, .. }) if reason == "no AAC audio"
        ));
    }

    #[test]
    fn a_broken_manifest_is_an_error() {
        assert!(matches!(
            Manifest::parse(HADES_URL, "<MPD"),
            Err(TrailerError::Xml { .. })
        ));
    }

    #[test]
    fn reads_iso_durations() {
        assert_eq!(
            iso_duration("PT1M25.5S"),
            Some(Duration::from_millis(85_500))
        );
        assert_eq!(iso_duration("PT3.0S"), Some(Duration::from_secs(3)));
        assert_eq!(iso_duration("PT1H2M3S"), Some(Duration::from_secs(3_723)));
        assert_eq!(iso_duration("P1DT1S"), Some(Duration::from_secs(86_401)));
        assert_eq!(iso_duration("PT12"), None);
        assert_eq!(iso_duration("1M25S"), None);
    }

    #[test]
    fn reads_the_number_templates() {
        assert_eq!(
            number_template("a-$Number%05d$.m4s"),
            Some(("a-".to_string(), 5, ".m4s".to_string()))
        );
        assert_eq!(
            number_template("a-$Number$.m4s"),
            Some(("a-".to_string(), 1, ".m4s".to_string()))
        );
        assert_eq!(number_template("a-$Time$.m4s"), None);
    }

    enum Answer {
        Body(u16, Vec<u8>),
        Never,
    }

    struct FakeCdn {
        answer: Answer,
        requests: Mutex<Vec<Request>>,
    }

    impl Fetch for FakeCdn {
        fn fetch(
            &self,
            request: Request,
        ) -> impl Future<Output = reqwest::Result<Response>> + Send {
            self.requests.lock().unwrap().push(request);
            let body = match &self.answer {
                Answer::Body(status, body) => Some((*status, body.clone())),
                Answer::Never => None,
            };

            async move {
                let Some((status, body)) = body else {
                    return std::future::pending().await;
                };
                Ok(http::Response::builder()
                    .status(status)
                    .body(body)
                    .unwrap()
                    .into())
            }
        }
    }

    fn downloader(answer: Answer) -> TrailerDownloader<FakeCdn> {
        TrailerDownloader::with_fetch(FakeCdn {
            answer,
            requests: Mutex::new(Vec::new()),
        })
    }

    #[tokio::test]
    async fn downloads_and_reads_a_manifest() {
        let downloader = downloader(Answer::Body(200, HADES.as_bytes().to_vec()));

        let manifest = downloader.manifest(HADES_URL).await.unwrap();

        assert_eq!(manifest, hades());
        let requests = downloader.fetch.requests.lock().unwrap();
        assert_eq!(requests[0].url().as_str(), HADES_URL);
        assert_eq!(requests[0].headers()[ACCEPT], MANIFEST_ACCEPT);
    }

    #[tokio::test]
    async fn downloads_a_segment() {
        let downloader = downloader(Answer::Body(200, vec![1, 2, 3]));
        let url = hades().audio.segments.url(2);

        assert_eq!(downloader.segment(&url).await.unwrap(), vec![1, 2, 3]);
        let requests = downloader.fetch.requests.lock().unwrap();
        assert_eq!(requests[0].url(), &url);
        assert_eq!(requests[0].headers()[ACCEPT], SEGMENT_ACCEPT);
    }

    #[tokio::test]
    async fn fails_on_a_missing_segment() {
        let downloader = downloader(Answer::Body(404, b"<html>".to_vec()));
        let url = hades().audio.segments.url(30);

        assert!(matches!(
            downloader.segment(&url).await,
            Err(TrailerError::Status {
                status: StatusCode::NOT_FOUND,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn refuses_a_segment_over_sixteen_megabytes() {
        let downloader = downloader(Answer::Body(200, vec![0; MAX_BYTES + 1]));
        let url = hades().video[0].segments.url(1);

        assert!(matches!(
            downloader.segment(&url).await,
            Err(TrailerError::TooLarge { .. })
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_on_a_cdn_that_does_not_answer() {
        let downloader = downloader(Answer::Never);
        let url = hades().video[0].segments.url(1);

        assert!(matches!(
            downloader.segment(&url).await,
            Err(TrailerError::Network { reason, .. }) if reason == "timed out"
        ));
    }

    #[tokio::test]
    async fn a_thread_outside_the_runtime_downloads_through_its_handle() {
        let downloader = downloader(Answer::Body(200, vec![7]));
        let url = hades().video[0].segments.url(1);
        let runtime = tokio::runtime::Handle::current();

        let bytes = tokio::task::spawn_blocking(move || runtime.block_on(downloader.segment(&url)))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(bytes, vec![7]);
    }
}
