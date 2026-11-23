use crate::{cancelled, check_cancel, cues::Word, CANCELLED};
use reqwest::{
    header::{HeaderValue, AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE},
    Client,
};
use serde::Deserialize;
use std::{path::Path, sync::atomic::AtomicBool, time::Duration};
use tokio_util::io::ReaderStream;
use zeroize::Zeroizing;

pub(crate) const MAX_AUDIO_BYTES: u64 = 20 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

pub(crate) struct Provider {
    client: Client,
    endpoint: String,
    authorization: HeaderValue,
    language: String,
}

impl Provider {
    pub fn new(key: &str, endpoint: &str, language: &str) -> Result<Self, String> {
        let secret = Zeroizing::new(format!("Token {key}"));
        let mut authorization =
            HeaderValue::from_str(&secret).map_err(|_| "Invalid Deepgram API key.".to_owned())?;
        authorization.set_sensitive(true);
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(660))
            .no_proxy()
            .build()
            .map_err(|_| "Cannot initialize secure transcription networking.".to_owned())?;
        Ok(Self {
            client,
            endpoint: endpoint.into(),
            authorization,
            language: language.into(),
        })
    }

    pub async fn words(
        &self,
        audio: &Path,
        duration: f64,
        cancel: &AtomicBool,
    ) -> Result<Vec<Word>, String> {
        check_cancel(cancel)?;
        let bytes = tokio::fs::metadata(audio)
            .await
            .map_err(|_| "Temporary subtitle audio is unavailable.".to_owned())?
            .len();
        if !(45..=MAX_AUDIO_BYTES).contains(&bytes) {
            return Err("Prepared subtitle audio is empty or exceeds 20 MiB.".into());
        }
        let file = tokio::fs::File::open(audio)
            .await
            .map_err(|_| "Cannot open temporary subtitle audio.".to_owned())?;
        let mut query = vec![
            ("model", "nova-3"),
            ("language", "multi"),
            ("punctuate", "true"),
            ("smart_format", "true"),
            ("mip_opt_out", "true"),
        ];
        if self.language != "auto" {
            query.push(("language_hint", self.language.as_str()));
        }
        let request = self
            .client
            .post(&self.endpoint)
            .query(&query)
            .header(AUTHORIZATION, self.authorization.clone())
            .header(CONTENT_TYPE, "audio/wav")
            .header(CONTENT_LENGTH, bytes)
            .body(reqwest::Body::wrap_stream(ReaderStream::new(file)));
        tokio::select! {
            biased;
            _ = cancelled(cancel) => Err(CANCELLED.into()),
            result = async {
                let mut response = request.send().await.map_err(network_error)?;
                let status = response.status();
                if !status.is_success() { return Err(status_error(status.as_u16())); }
                if response.content_length().is_some_and(|length| length > MAX_RESPONSE_BYTES as u64) {
                    return Err("Transcription response exceeds the supported size.".into());
                }
                let mut body = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(network_error)? {
                    if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                        return Err("Transcription response exceeds the supported size.".into());
                    }
                    body.extend_from_slice(&chunk);
                }
                parse(&body, duration)
            } => result,
        }
    }
}

fn network_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "Transcription timed out. Check your connection and try a shorter range.".into()
    } else {
        "Could not communicate securely with Deepgram. Check your connection and try again.".into()
    }
}

fn status_error(status: u16) -> String {
    match status {
        401 | 403 => "Deepgram rejected the API key or its permissions.".into(),
        402 => "Deepgram requires available account credit.".into(),
        413 => "Deepgram rejected the audio upload size.".into(),
        429 => "Deepgram rate limit or account quota reached. Wait and check your account before retrying.".into(),
        300..=399 => "Deepgram returned an unexpected redirect; no credentials were forwarded.".into(),
        500..=599 => "Deepgram could not complete the transcription. Try again later.".into(),
        _ => format!("Deepgram rejected the transcription request (HTTP {status})."),
    }
}

#[derive(Deserialize)]
struct Response {
    results: Results,
}
#[derive(Deserialize)]
struct Results {
    channels: Vec<Channel>,
}
#[derive(Deserialize)]
struct Channel {
    alternatives: Vec<Alternative>,
}
#[derive(Deserialize)]
struct Alternative {
    transcript: String,
    words: Vec<ProviderWord>,
}
#[derive(Deserialize)]
struct ProviderWord {
    start: f64,
    end: f64,
    word: String,
    punctuated_word: Option<String>,
}

pub(crate) fn parse(bytes: &[u8], duration: f64) -> Result<Vec<Word>, String> {
    let mut response: Response = serde_json::from_slice(bytes)
        .map_err(|_| "Deepgram returned an invalid transcription response.".to_owned())?;
    if response.results.channels.len() != 1 {
        return Err("Deepgram returned an unexpected audio channel count.".into());
    }
    let channel = response.results.channels.pop().unwrap();
    let alternative = channel
        .alternatives
        .into_iter()
        .next()
        .ok_or("Deepgram returned no transcription result.")?;
    if alternative.words.len() > 20_000 {
        return Err("Deepgram returned too many words for one audio chunk.".into());
    }
    if alternative.words.is_empty() && !alternative.transcript.trim().is_empty() {
        return Err(
            "Deepgram returned text without word timestamps; subtitles were not changed.".into(),
        );
    }
    let mut previous = 0.0;
    let mut words = Vec::with_capacity(alternative.words.len());
    for word in alternative.words {
        let text = word.punctuated_word.unwrap_or(word.word).trim().to_owned();
        if !word.start.is_finite()
            || !word.end.is_finite()
            || word.start < 0.0
            || word.start >= duration
            || word.end <= word.start
            || word.end > duration + 0.1
            || word.start < previous
            || text.is_empty()
            || text.chars().count() > 500
            || text.chars().any(char::is_control)
        {
            return Err(
                "Deepgram returned invalid subtitle timing or text; subtitles were not changed."
                    .into(),
            );
        }
        previous = word.start;
        if word.end > word.start {
            words.push(Word {
                start: word.start,
                end: word.end.min(duration),
                text,
            });
        }
    }
    Ok(words)
}
