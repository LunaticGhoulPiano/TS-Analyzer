# Copy to windows.local.psd1 (Git-ignored) only for machine-specific overrides.
# Paths may be absolute or relative to the repository, never relative to the current shell.
# Empty tool paths enable discovery from PATH / standard installation locations.
@{
  GStreamerRoot = ''
  TSDuckRoot = ''
  TexRoot = ''
  MsvcRuntime = ''
  MsvcLicenses = '' # Auto-discovered from the selected runtime; override for a relocated SDK.
  InnoCompiler = ''
  # A representative complete report's .fls, required only by Package.
  TexRecorder = ''
  # Optional existing unpacked package, used by Installer / VerifyRuntime / VerifyReports.
  Package = ''
  Recordings = 'developmentHelpers/test-data/inputs/local'
  OutputDirectory = 'outputs/windows'
  # Deploy creates a platform/version/architecture folder beneath this root.
  DeployDirectory = 'outputs/deploy/windows'
}
