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
        ("nav.meetings", "Meetings & transcripts"),
        ("nav.search", "Search meetings…"),
        ("nav.settings", "Settings"),
        ("nav.import", "Import audio"),
        ("nav.new", "New meeting"),
        ("nav.delete", "Delete meeting"),
        ("nav.rename", "Rename meeting"),
        ("sidebar.today", "Today"),
        ("sidebar.this_week", "This week"),
        ("sidebar.earlier", "Earlier"),
        ("sidebar.no_matches", "No meetings match that."),
        ("sidebar.clear_search", "Clear search"),
        ("sidebar.empty", "Nothing recorded yet."),
        ("sidebar.empty_cta", "Import audio"),
        ("record.start", "Record"),
        ("record.pause", "Pause"),
        ("record.resume", "Resume"),
        ("record.stop", "Stop"),
        ("record.recording_badge", "Recording"),
        ("record.paused_badge", "Paused"),
        ("shortcut.unavailable", "Ctrl+Shift+R is taken by another app, so it only works while Vesper is focused."),
        ("processing.saving", "Saving the recording…"),
        ("processing.transcribing", "Transcribing…"),
        ("processing.summarizing", "Writing the summary…"),
        ("processing.summary_failed", "The recording is saved — the summary failed."),
        ("dock.system_default", "System default"),
        ("settings.local_stt", "Local speech-to-text"),
        ("settings.local_llm", "Local language model"),
        ("settings.no_local_stt", "No speech-to-text model installed yet."),
        ("settings.no_local_llm", "No language model installed yet."),
        ("action.summarize", "Summarize"),
        ("action.retranscribe", "Retranscribe"),
        ("action.dismiss", "Dismiss"),
        ("action.retry", "Retry"),
        ("action.copy", "Copy"),
        ("action.discard", "Discard"),
        ("meeting.cost", "Spent on cloud models for this meeting"),
        ("summary.improve", "Improve"),
        ("summary.improving", "Improving…"),
        ("summary.history", "Versions"),
        ("summary.restore", "Restore"),
        ("summary.current", "Current"),
        ("summary.origin.summarize", "First summary"),
        ("summary.origin.key_points", "Key points improved"),
        ("summary.origin.action_items", "Action items improved"),
        ("summary.origin.restore", "Restored"),
        ("overlay.listening", "Listening…"),
        ("window.minimize", "Minimize"),
        ("window.maximize", "Maximize"),
        ("window.restore", "Restore down"),
        ("window.close", "Close"),
        ("action.copied", "Copied"),
        ("action.close", "Close"),
        ("download.connecting", "Connecting…"),
        ("download.downloading", "Downloading"),
        ("download.verifying", "Verifying…"),
        ("download.extracting", "Extracting…"),
        ("download.done", "Done"),
        ("download.retrying", "Retrying — attempt {attempt}, resuming at {at}"),
        ("download.failed", "Download failed"),
        ("picker.search", "Search models…"),
        ("picker.recent", "Recent"),
        ("picker.all", "All models"),
        ("picker.no_match", "No model matches “{query}”."),
        ("picker.clear", "Clear"),
        ("picker.needs_key", "Add an OpenRouter API key to load the full list."),
        ("picker.fetch_failed", "Could not load the model list."),
        ("picker.unknown_model", "Not in the catalogue"),
        ("section.summary", "Summary"),
        ("section.key_points", "Key points"),
        ("section.action_items", "Action items"),
        ("model.ready", "Ready"),
        ("model.not_downloaded", "Not downloaded"),
        ("model.download", "Download"),
        ("model.unverified", "Downloaded, not verified yet"),
        ("model.verify", "Verify"),
        // Deliberately not "will resume": a part-file written before the ETag
        // sidecar existed carries no validator, so the server may answer from
        // zero. This states what is on disk, which is always true.
        ("model.partial", "{done} of {total} already downloaded"),
        ("model.continue", "Continue"),
        ("gate.fix", "Open settings"),
        ("live.stt_failing", "Transcription is failing. The recording is still being captured and saved."),
        ("settings.saved", "Saved"),
        ("gate.unavailable", "Could not check whether recording is possible."),
        ("level.me", "Me"),
        ("level.others", "Others"),
        ("onboarding.default_mic", "Default microphone"),
        ("onboarding.default_system", "Default system audio"),
        ("confirm.cancel", "Cancel"),
        ("confirm.record_title", "Let the room know"),
        ("confirm.record_body", "Vesper is about to record and transcribe this meeting. Tell the other people in it before you start — in many places you are required to."),
        ("confirm.record_accept", "Start recording"),
        ("confirm.record_dont_ask", "Don't remind me again"),
        ("confirm.delete_title", "Delete this meeting?"),
        ("confirm.delete_body", "The transcript, the summary and the audio recording are all deleted from this computer. This cannot be undone."),
        ("confirm.delete_accept", "Delete"),
        ("tab.transcript", "Transcript"),
        ("tab.summary", "Summary"),
        ("update.available", "Version {version} is available."),
        ("update.install", "Install and restart"),
        ("update.later", "Later"),
        ("update.installing", "Downloading the update…"),
        ("update.relaunching", "Update installed. Restarting…"),
        ("empty.ready", "Ready when you are"),
        ("empty.ready_body", "Start a recording, import audio, or open a past meeting. Everything stays on this machine."),
        ("empty.transcript", "No transcript yet"),
        ("empty.transcript_body", "Hit Record to capture dual-channel audio. Live lines appear as Me / Others."),
        ("empty.summary", "No summary yet"),
        ("empty.summary_body", "Summarize this meeting to get an overview, the key points and the action items."),
        ("gate.onboarding", "Finish onboarding before recording."),
        ("gate.local_stt", "Local STT model is not installed. Download it or switch to OpenRouter."),
        ("gate.local_stt_unverified", "The local model is downloaded but not verified yet. Verify it in settings to start recording."),
        ("gate.openrouter_key", "OpenRouter API key is required for cloud STT."),
        ("gate.openrouter_stt_model", "Select an OpenRouter STT model."),
        ("onboarding.title", "Welcome to Vesper"),
        ("onboarding.language", "Interface language"),
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
        ("settings.models", "Models"),
        ("settings.devices", "Devices"),
        ("settings.appearance", "Appearance"),
        // Qualified because there are two languages in this app now: the one the
        // interface is written in, and the one the audio is spoken in.
        ("settings.language", "Interface language"),
        ("settings.transcription_language", "Transcription language"),
        ("language.auto", "Detect automatically"),
        ("settings.theme", "Theme"),
        ("theme.light", "Light"),
        ("theme.dark", "Dark"),
        ("settings.capabilities", "This device"),
        ("settings.save", "Save settings"),
        ("settings.stt_provider", "STT provider"),
        ("settings.llm_provider", "LLM provider"),
        ("settings.reasoning", "Reasoning / CoT (OpenRouter)"),
        ("settings.auto_summarize", "Auto-summarize after recording"),
        ("settings.pick_stt_model", "OpenRouter STT model"),
        ("settings.pick_llm_model", "OpenRouter LLM model"),
        ("settings.compute_backend", "Where local models run"),
        ("backend.auto", "Automatic"),
        ("backend.cuda", "NVIDIA (CUDA)"),
        ("backend.vulkan", "Any GPU (Vulkan)"),
        ("backend.cpu", "CPU only"),
        ("backend.unavailable", "not available in this build"),
        ("cap.cuda", "GPU"),
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
        ("nav.meetings", "Reuniões e transcrições"),
        ("nav.search", "Buscar reuniões…"),
        ("nav.settings", "Configurações"),
        ("nav.import", "Importar áudio"),
        ("nav.new", "Nova reunião"),
        ("nav.delete", "Excluir reunião"),
        ("nav.rename", "Renomear reunião"),
        ("sidebar.today", "Hoje"),
        ("sidebar.this_week", "Esta semana"),
        ("sidebar.earlier", "Antes"),
        ("sidebar.no_matches", "Nenhuma reunião corresponde."),
        ("sidebar.clear_search", "Limpar busca"),
        ("sidebar.empty", "Nada gravado ainda."),
        ("sidebar.empty_cta", "Importar áudio"),
        ("record.start", "Gravar"),
        ("record.pause", "Pausar"),
        ("record.resume", "Retomar"),
        ("record.stop", "Parar"),
        ("record.recording_badge", "Gravando"),
        ("record.paused_badge", "Pausado"),
        ("shortcut.unavailable", "Ctrl+Shift+R está em uso por outro aplicativo, então só funciona com o Vesper em foco."),
        ("processing.saving", "Salvando a gravação…"),
        ("processing.transcribing", "Transcrevendo…"),
        ("processing.summarizing", "Escrevendo o resumo…"),
        ("processing.summary_failed", "A gravação está salva — o resumo falhou."),
        ("dock.system_default", "Padrão do sistema"),
        ("settings.local_stt", "Transcrição local"),
        ("settings.local_llm", "Modelo de linguagem local"),
        ("settings.no_local_stt", "Nenhum modelo de transcrição instalado ainda."),
        ("settings.no_local_llm", "Nenhum modelo de linguagem instalado ainda."),
        ("action.summarize", "Resumir"),
        ("action.retranscribe", "Transcrever de novo"),
        ("action.dismiss", "Dispensar"),
        ("action.retry", "Tentar de novo"),
        ("action.copy", "Copiar"),
        ("action.discard", "Descartar"),
        ("meeting.cost", "Gasto com modelos de nuvem nesta reunião"),
        ("summary.improve", "Melhorar"),
        ("summary.improving", "Melhorando…"),
        ("summary.history", "Versões"),
        ("summary.restore", "Restaurar"),
        ("summary.current", "Atual"),
        ("summary.origin.summarize", "Primeiro resumo"),
        ("summary.origin.key_points", "Pontos principais melhorados"),
        ("summary.origin.action_items", "Ações melhoradas"),
        ("summary.origin.restore", "Restaurada"),
        ("overlay.listening", "Ouvindo…"),
        ("window.minimize", "Minimizar"),
        ("window.maximize", "Maximizar"),
        ("window.restore", "Restaurar"),
        ("window.close", "Fechar"),
        ("action.copied", "Copiado"),
        ("action.close", "Fechar"),
        ("download.connecting", "Conectando…"),
        ("download.downloading", "Baixando"),
        ("download.verifying", "Verificando…"),
        ("download.extracting", "Extraindo…"),
        ("download.done", "Concluído"),
        ("download.retrying", "Nova tentativa — {attempt}ª, retomando em {at}"),
        ("download.failed", "Falha no download"),
        ("picker.search", "Buscar modelos…"),
        ("picker.recent", "Recentes"),
        ("picker.all", "Todos os modelos"),
        ("picker.no_match", "Nenhum modelo corresponde a “{query}”."),
        ("picker.clear", "Limpar"),
        (
            "picker.needs_key",
            "Adicione uma chave da API OpenRouter para carregar a lista completa.",
        ),
        ("picker.fetch_failed", "Não foi possível carregar a lista de modelos."),
        ("picker.unknown_model", "Fora do catálogo"),
        ("section.summary", "Resumo"),
        ("section.key_points", "Pontos principais"),
        ("section.action_items", "Ações"),
        ("model.ready", "Pronto"),
        ("model.not_downloaded", "Não baixado"),
        ("model.download", "Baixar"),
        ("model.unverified", "Baixado, ainda não verificado"),
        ("model.verify", "Verificar"),
        ("model.partial", "{done} de {total} já baixados"),
        ("model.continue", "Continuar"),
        ("gate.fix", "Abrir configurações"),
        ("live.stt_failing", "A transcrição está falhando. A gravação continua sendo capturada e salva."),
        ("settings.saved", "Salvo"),
        ("gate.unavailable", "Não foi possível verificar se dá para gravar."),
        ("level.me", "Eu"),
        ("level.others", "Outros"),
        ("onboarding.default_mic", "Microfone padrão"),
        ("onboarding.default_system", "Áudio padrão do sistema"),
        ("confirm.cancel", "Cancelar"),
        ("confirm.record_title", "Avise a sala"),
        ("confirm.record_body", "O Vesper vai gravar e transcrever esta reunião. Avise as outras pessoas antes de começar — em muitos lugares isso é exigido por lei."),
        ("confirm.record_accept", "Começar a gravar"),
        ("confirm.record_dont_ask", "Não lembrar de novo"),
        ("confirm.delete_title", "Excluir esta reunião?"),
        ("confirm.delete_body", "A transcrição, o resumo e a gravação de áudio são apagados deste computador. Não dá para desfazer."),
        ("confirm.delete_accept", "Excluir"),
        ("tab.transcript", "Transcrição"),
        ("tab.summary", "Resumo"),
        ("update.available", "A versão {version} está disponível."),
        ("update.install", "Instalar e reiniciar"),
        ("update.later", "Depois"),
        ("update.installing", "Baixando a atualização…"),
        ("update.relaunching", "Atualização instalada. Reiniciando…"),
        ("empty.ready", "Pronto quando você estiver"),
        ("empty.ready_body", "Inicie uma gravação, importe áudio ou abra uma reunião. Tudo fica neste computador."),
        ("empty.transcript", "Ainda sem transcrição"),
        ("empty.transcript_body", "Toque em Gravar para capturar áudio dual. Linhas ao vivo como Eu / Outros."),
        ("empty.summary", "Ainda sem resumo"),
        ("empty.summary_body", "Resuma esta reunião para ter uma visão geral, os pontos principais e as ações."),
        ("gate.onboarding", "Conclua a configuração inicial antes de gravar."),
        ("gate.local_stt", "Modelo STT local não instalado. Baixe-o ou use OpenRouter."),
        ("gate.local_stt_unverified", "O modelo local está baixado mas ainda não verificado. Verifique nas configurações para poder gravar."),
        ("gate.openrouter_key", "Chave da API OpenRouter é necessária para STT na nuvem."),
        ("gate.openrouter_stt_model", "Selecione um modelo STT da OpenRouter."),
        ("onboarding.title", "Bem-vindo ao Vesper"),
        ("onboarding.language", "Idioma da interface"),
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
        ("settings.models", "Modelos"),
        ("settings.devices", "Dispositivos"),
        ("settings.appearance", "Aparência"),
        ("settings.language", "Idioma da interface"),
        ("settings.transcription_language", "Idioma da transcrição"),
        ("language.auto", "Detectar automaticamente"),
        ("settings.theme", "Tema"),
        ("theme.light", "Claro"),
        ("theme.dark", "Escuro"),
        ("settings.capabilities", "Este dispositivo"),
        ("settings.save", "Salvar"),
        ("settings.stt_provider", "Provedor de STT"),
        ("settings.llm_provider", "Provedor de LLM"),
        ("settings.reasoning", "Raciocínio / CoT (OpenRouter)"),
        ("settings.auto_summarize", "Resumir automaticamente após gravar"),
        ("settings.pick_stt_model", "Modelo STT OpenRouter"),
        ("settings.pick_llm_model", "Modelo LLM OpenRouter"),
        ("settings.compute_backend", "Onde os modelos locais rodam"),
        ("backend.auto", "Automático"),
        ("backend.cuda", "NVIDIA (CUDA)"),
        ("backend.vulkan", "Qualquer GPU (Vulkan)"),
        ("backend.cpu", "Somente CPU"),
        ("backend.unavailable", "indisponível nesta versão"),
        ("cap.cuda", "GPU"),
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

    /// A key present in one locale and missing in the other reaches the user as
    /// the raw key, because lookup falls back to the key itself — which is how a
    /// heading ends up reading `sidebar.today`.
    #[test]
    fn both_locales_carry_the_same_keys() {
        let en = catalog(Locale::En);
        let pt = catalog(Locale::PtBr);
        let mut missing: Vec<String> = en
            .keys()
            .filter(|k| !pt.contains_key(*k))
            .map(|k| format!("only in en: {k}"))
            .collect();
        missing.extend(
            pt.keys()
                .filter(|k| !en.contains_key(*k))
                .map(|k| format!("only in pt: {k}")),
        );
        missing.sort();
        assert!(missing.is_empty(), "{missing:?}");
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
