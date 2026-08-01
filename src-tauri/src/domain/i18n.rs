//! Minimal i18n dictionaries EN + PT-BR.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Locale {
    #[default]
    En,
    PtBr,
}

impl Locale {
    pub fn from_code(code: &str) -> Self {
        match code {
            "pt" | "pt-BR" | "pt_br" | "pt-br" => Locale::PtBr,
            _ => Locale::En,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::PtBr => "pt-BR",
        }
    }
}

fn en_dict() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("app.name", "Vesper"),
        ("app.tagline", "Private meeting notes"),
        ("nav.meetings", "Meetings"),
        ("nav.search", "Search meetings…"),
        ("nav.settings", "Settings"),
        ("record.start", "Record"),
        ("record.pause", "Pause"),
        ("record.resume", "Resume"),
        ("record.stop", "Stop"),
        ("tab.transcript", "Transcript"),
        ("tab.summary", "Summary"),
        ("tab.chat", "Chat"),
        ("update.available", "Version {version} is available."),
        ("update.install", "Install and restart"),
        ("update.later", "Later"),
        ("update.installing", "Downloading the update…"),
        ("update.relaunching", "Update installed. Restarting…"),
        ("empty.ready", "Ready when you are"),
        ("empty.ready_body", "Start a recording, import audio, or open a past meeting. Everything stays on this machine."),
        ("empty.transcript", "No transcript yet"),
        ("empty.transcript_body", "Hit Record to capture dual-channel audio. Live lines appear as Me / Others."),
        ("gate.onboarding", "Finish onboarding before recording."),
        ("gate.local_stt", "Local STT model is not installed. Download it or switch to OpenRouter."),
        ("gate.openrouter_key", "OpenRouter API key is required for cloud STT."),
        ("gate.openrouter_stt_model", "Select an OpenRouter STT model."),
        ("onboarding.title", "Welcome to Vesper"),
        ("onboarding.language", "Language"),
        ("onboarding.stt_path", "Speech-to-text"),
        ("onboarding.llm_path", "Assistant model"),
        ("onboarding.devices", "Audio devices"),
        ("onboarding.mic", "Microphone"),
        ("onboarding.system", "System / computer audio"),
        ("onboarding.api_key", "OpenRouter API key"),
        ("onboarding.local_models", "Local models"),
        ("onboarding.finish", "Finish setup"),
        ("onboarding.next", "Continue"),
        ("onboarding.back", "Back"),
        ("settings.title", "Settings"),
        ("settings.local", "Local"),
        ("settings.cloud", "OpenRouter"),
        ("settings.devices", "Devices"),
        ("settings.language", "Language"),
        ("settings.capabilities", "This device"),
        ("settings.save", "Save settings"),
        ("settings.stt_provider", "STT provider"),
        ("settings.llm_provider", "LLM provider"),
        ("settings.reasoning", "Reasoning / CoT (OpenRouter)"),
        ("settings.auto_summarize", "Auto-summarize after recording"),
        ("settings.pick_stt_model", "OpenRouter STT model"),
        ("settings.pick_llm_model", "OpenRouter LLM model"),
        ("cap.cuda", "CUDA / GPU"),
        ("cap.cpu", "CPU"),
        ("cap.recommended", "Recommended setup"),
        ("speaker.me", "Me"),
        ("speaker.others", "Others"),
    ])
}

fn pt_dict() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("app.name", "Vesper"),
        ("app.tagline", "Notas de reunião privadas"),
        ("nav.meetings", "Reuniões"),
        ("nav.search", "Buscar reuniões…"),
        ("nav.settings", "Configurações"),
        ("record.start", "Gravar"),
        ("record.pause", "Pausar"),
        ("record.resume", "Retomar"),
        ("record.stop", "Parar"),
        ("tab.transcript", "Transcrição"),
        ("tab.summary", "Resumo"),
        ("tab.chat", "Chat"),
        ("update.available", "A versão {version} está disponível."),
        ("update.install", "Instalar e reiniciar"),
        ("update.later", "Depois"),
        ("update.installing", "Baixando a atualização…"),
        ("update.relaunching", "Atualização instalada. Reiniciando…"),
        ("empty.ready", "Pronto quando você estiver"),
        ("empty.ready_body", "Inicie uma gravação, importe áudio ou abra uma reunião. Tudo fica neste computador."),
        ("empty.transcript", "Ainda sem transcrição"),
        ("empty.transcript_body", "Toque em Gravar para capturar áudio dual. Linhas ao vivo como Eu / Outros."),
        ("gate.onboarding", "Conclua a configuração inicial antes de gravar."),
        ("gate.local_stt", "Modelo STT local não instalado. Baixe-o ou use OpenRouter."),
        ("gate.openrouter_key", "Chave da API OpenRouter é necessária para STT na nuvem."),
        ("gate.openrouter_stt_model", "Selecione um modelo STT da OpenRouter."),
        ("onboarding.title", "Bem-vindo ao Vesper"),
        ("onboarding.language", "Idioma"),
        ("onboarding.stt_path", "Transcrição de voz"),
        ("onboarding.llm_path", "Modelo do assistente"),
        ("onboarding.devices", "Dispositivos de áudio"),
        ("onboarding.mic", "Microfone"),
        ("onboarding.system", "Áudio do sistema / computador"),
        ("onboarding.api_key", "Chave API OpenRouter"),
        ("onboarding.local_models", "Modelos locais"),
        ("onboarding.finish", "Concluir"),
        ("onboarding.next", "Continuar"),
        ("onboarding.back", "Voltar"),
        ("settings.title", "Configurações"),
        ("settings.local", "Local"),
        ("settings.cloud", "OpenRouter"),
        ("settings.devices", "Dispositivos"),
        ("settings.language", "Idioma"),
        ("settings.capabilities", "Este dispositivo"),
        ("settings.save", "Salvar"),
        ("settings.stt_provider", "Provedor de STT"),
        ("settings.llm_provider", "Provedor de LLM"),
        ("settings.reasoning", "Raciocínio / CoT (OpenRouter)"),
        ("settings.auto_summarize", "Resumir automaticamente após gravar"),
        ("settings.pick_stt_model", "Modelo STT OpenRouter"),
        ("settings.pick_llm_model", "Modelo LLM OpenRouter"),
        ("cap.cuda", "CUDA / GPU"),
        ("cap.cpu", "CPU"),
        ("cap.recommended", "Configuração recomendada"),
        ("speaker.me", "Eu"),
        ("speaker.others", "Outros"),
    ])
}

/// Translate `key` for `locale`. Falls back to English, then the key itself.
pub fn t(locale: Locale, key: &str) -> String {
    let primary = match locale {
        Locale::En => en_dict(),
        Locale::PtBr => pt_dict(),
    };
    if let Some(v) = primary.get(key) {
        return (*v).to_string();
    }
    if locale != Locale::En {
        if let Some(v) = en_dict().get(key) {
            return (*v).to_string();
        }
    }
    key.to_string()
}

/// Full catalog for frontend hydration.
pub fn catalog(locale: Locale) -> HashMap<String, String> {
    let src = match locale {
        Locale::En => en_dict(),
        Locale::PtBr => pt_dict(),
    };
    src.into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn en_and_pt_differ_on_core_keys() {
        let en = t(Locale::En, "record.start");
        let pt = t(Locale::PtBr, "record.start");
        assert_eq!(en, "Record");
        assert_eq!(pt, "Gravar");
        assert_ne!(en, pt);

        let en2 = t(Locale::En, "onboarding.title");
        let pt2 = t(Locale::PtBr, "onboarding.title");
        assert!(en2.contains("Welcome"));
        assert!(pt2.contains("Bem-vindo"));
        assert_ne!(en2, pt2);
    }

    #[test]
    fn locale_from_code() {
        assert_eq!(Locale::from_code("pt-BR"), Locale::PtBr);
        assert_eq!(Locale::from_code("en"), Locale::En);
    }

    #[test]
    fn catalog_non_empty_both() {
        assert!(catalog(Locale::En).len() > 10);
        assert!(catalog(Locale::PtBr).len() > 10);
    }
}
