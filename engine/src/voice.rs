//! Voice/TTS — Speech-to-Text and Text-to-Speech integration.
//!
//! Provides types and configuration for voice-based interactions.
//! Actual STT/TTS calls go through external provider APIs.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Speech-to-Text provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum STTProvider {
    Whisper,
    Google,
    Azure,
    Deepgram,
}

/// Text-to-Speech provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TTSProvider {
    ElevenLabs,
    OpenAiTts,
    GoogleTts,
    Coqui,
}

/// Voice session configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceConfig {
    pub stt_provider: STTProvider,
    pub tts_provider: TTSProvider,
    pub voice_id: Option<String>,
    pub language: String,
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
}

fn default_sample_rate() -> u32 {
    16000
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            stt_provider: STTProvider::Whisper,
            tts_provider: TTSProvider::OpenAiTts,
            voice_id: None,
            language: "en".into(),
            sample_rate: 16000,
        }
    }
}

/// Voice session state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceState {
    Idle,
    Listening,
    Processing,
    Speaking,
}

/// Result of STT transcription.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionResult {
    pub text: String,
    pub language: String,
    pub confidence: f64,
    pub duration_seconds: f64,
}

/// Result of TTS synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisResult {
    pub audio_format: String,
    pub duration_seconds: f64,
    pub sample_rate: u32,
    /// Audio data (would be bytes in production, represented as length here).
    pub audio_size_bytes: usize,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_config_defaults() {
        let config = VoiceConfig::default();
        assert_eq!(config.stt_provider, STTProvider::Whisper);
        assert_eq!(config.tts_provider, TTSProvider::OpenAiTts);
        assert_eq!(config.language, "en");
        assert_eq!(config.sample_rate, 16000);
    }

    #[test]
    fn voice_config_serde() {
        let config = VoiceConfig {
            stt_provider: STTProvider::Deepgram,
            tts_provider: TTSProvider::ElevenLabs,
            voice_id: Some("rachel".into()),
            language: "es".into(),
            sample_rate: 44100,
        };
        let json = serde_json::to_string(&config).unwrap();
        let back: VoiceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.stt_provider, STTProvider::Deepgram);
        assert_eq!(back.voice_id, Some("rachel".into()));
    }

    #[test]
    fn voice_state_transitions() {
        let states = vec![
            VoiceState::Idle,
            VoiceState::Listening,
            VoiceState::Processing,
            VoiceState::Speaking,
        ];
        for state in states {
            let json = serde_json::to_string(&state).unwrap();
            let back: VoiceState = serde_json::from_str(&json).unwrap();
            assert_eq!(back, state);
        }
    }

    #[test]
    fn all_providers_serialize() {
        let stt = vec![STTProvider::Whisper, STTProvider::Google, STTProvider::Azure, STTProvider::Deepgram];
        for p in stt {
            let json = serde_json::to_string(&p).unwrap();
            let _: STTProvider = serde_json::from_str(&json).unwrap();
        }

        let tts = vec![TTSProvider::ElevenLabs, TTSProvider::OpenAiTts, TTSProvider::GoogleTts, TTSProvider::Coqui];
        for p in tts {
            let json = serde_json::to_string(&p).unwrap();
            let _: TTSProvider = serde_json::from_str(&json).unwrap();
        }
    }
}
