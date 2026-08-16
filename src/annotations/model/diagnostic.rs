#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticDisposition {
    Normalized,
    WouldDrop,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteroperabilityDiagnostic {
    code: String,
    severity: DiagnosticSeverity,
    path: String,
    disposition: DiagnosticDisposition,
    message: String,
}

impl InteroperabilityDiagnostic {
    pub(crate) fn new(
        code: impl Into<String>,
        severity: DiagnosticSeverity,
        path: impl Into<String>,
        disposition: DiagnosticDisposition,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            severity,
            path: path.into(),
            disposition,
            message: message.into(),
        }
    }

    pub(crate) fn normalized(
        code: impl Into<String>,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(
            code,
            DiagnosticSeverity::Info,
            path,
            DiagnosticDisposition::Normalized,
            message,
        )
    }

    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    #[must_use]
    pub const fn severity(&self) -> DiagnosticSeverity {
        self.severity
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub const fn disposition(&self) -> DiagnosticDisposition {
        self.disposition
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub const fn blocks_roundtrip(&self) -> bool {
        matches!(
            self.disposition,
            DiagnosticDisposition::WouldDrop | DiagnosticDisposition::Unsupported
        )
    }
}
