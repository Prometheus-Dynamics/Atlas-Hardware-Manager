<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { Panel } from "$lib/components";
  import { resetPageHeader, setPageHeader } from "$lib/stores/pageHeader";

  interface DiscoveredDevice {
    id: string;
    displayName: string;
    status: string;
    connectionChips: string[];
    ipAddress?: string | null;
    macAddress?: string | null;
    interfaceName?: string | null;
    usbLocation?: string | null;
    vendorProduct?: string | null;
    runtimeProduct?: string | null;
    firmwareVersion?: string | null;
    osVersion?: string | null;
    telemetrySummary?: string | null;
    detail: string;
  }

  interface DeviceDiscoverySnapshot {
    generatedAtEpochMs: number;
    devices: DiscoveredDevice[];
    warnings: string[];
  }

  interface DeviceDiscoveryProgressEvent {
    generatedAtEpochMs: number;
    devices: DiscoveredDevice[];
    warnings: string[];
    inProgress: boolean;
  }

  interface ReleaseImageOption {
    releaseTag: string;
    releaseName: string;
    prerelease: boolean;
    assetName: string;
    downloadUrl: string;
    sizeBytes: number;
    publishedAt?: string | null;
  }

  interface ReleaseInstallResult {
    success: boolean;
    mode: string;
    imagePath?: string | null;
    selectedTargetPath?: string | null;
    rpiboot?: OperationResult | null;
    flash?: OperationResult | null;
    message: string;
  }

  interface OperationResult {
    success: boolean;
    exitCode?: number | null;
    durationMs?: number | null;
    stdout?: string | null;
    stderr?: string | null;
    message?: string | null;
    timedOut?: boolean;
  }

  interface InstallLogEntry {
    timestamp: number;
    level: "info" | "warn" | "error";
    text: string;
  }

  interface UpdaterProgressEvent {
    runId?: string | null;
    mode: string;
    step: string;
    status: "running" | "success" | "error" | "skipped" | "info";
    message: string;
    timestampEpochMs: number;
    stdout?: string | null;
    stderr?: string | null;
    exitCode?: number | null;
    durationMs?: number | null;
    imagePath?: string | null;
    targetPath?: string | null;
    progressPercent?: number | null;
    bytesWritten?: number | null;
    bytesTotal?: number | null;
  }

  type UpdaterStepStatus = "pending" | "active" | "done" | "error" | "skipped";

  interface UpdaterStep {
    key: string;
    label: string;
    detail: string;
    status: UpdaterStepStatus;
  }

  interface DeviceFact {
    label: string;
    value: string;
  }

  interface DeviceLogResult {
    success: boolean;
    sourceUrl?: string | null;
    lineCount: number;
    truncated: boolean;
    fetchedAtEpochMs: number;
    logText: string;
    message: string;
  }

  interface DeviceTelemetryEvent {
    streamId: string;
    status: "connecting" | "data" | "error" | "stopped";
    message: string;
    timestampEpochMs: number;
    runtimeProduct?: string | null;
    osVersion?: string | null;
    telemetrySummary?: string | null;
    sourceUrl?: string | null;
  }

  interface WpilibLogEntry {
    id: string;
    fileName: string;
    remotePath: string;
    sizeBytes: number;
    modifiedEpochMs?: number | null;
  }

  interface WpilibLogListResult {
    success: boolean;
    fetchedAtEpochMs: number;
    entries: WpilibLogEntry[];
    message: string;
  }

  interface WpilibLogSelection {
    fileName: string;
    remotePath: string;
  }

  interface WpilibLogActionItemResult {
    fileName: string;
    remotePath: string;
    localPath?: string | null;
    success: boolean;
    message: string;
  }

  interface WpilibLogActionResult {
    success: boolean;
    processedCount: number;
    successCount: number;
    failedCount: number;
    completedAtEpochMs: number;
    downloadDirectory?: string | null;
    results: WpilibLogActionItemResult[];
    message: string;
  }
  
  interface UpdaterRecoverySession {
    runId: string;
    mode: string;
    targetIpAddress?: string | null;
    expectedUpdateId?: string | null;
    startedAtEpochMs: number;
    resumedAtEpochMs: number;
    note?: string | null;
  }

  interface ExistingOtaUpdateProbeResult {
    targetIpAddress: string;
    updateId?: string | null;
    stage?: string | null;
    progressPercent?: number | null;
    lastError?: string | null;
  }

  interface PowerCycleReminder {
    runId?: string | null;
    deviceId?: string | null;
    message: string;
    createdAtEpochMs: number;
  }

  interface InstallDeviceHint {
    usbLocation?: string | null;
    macAddress?: string | null;
    ipAddress?: string | null;
    vendorProduct?: string | null;
    runtimeProduct?: string | null;
    displayName?: string | null;
  }

  type OtaApplyTransport = "api" | "usb";

  let devices: DiscoveredDevice[] = [];
  let selectedDeviceId: string | null = null;
  let selectedDevice: DiscoveredDevice | null = null;
  let warnings: string[] = [];
  let discoveryBusy = false;
  let discoveryInFlight = false;
  let discoveryManualQueued = false;
  let discoveryRequestGeneration = 0;
  let discoveryError: string | null = null;
  let actionError: string | null = null;
  let lastDiscoveryEpochMs: number | null = null;
  let showUpdateModal = false;
  let releaseOptions: ReleaseImageOption[] = [];
  let releasesLoading = false;
  let releasesError: string | null = null;
  let installAction: "flash" | "mount" | "ota" = "flash";
  let otaApplyMenuOpen = false;
  let imageSource: "release" | "local" = "release";
  let selectedReleaseUrl = "";
  let localImagePath = "";
  let localImageFileName = "";
  let installBusy = false;
  let installError: string | null = null;
  let installResult: ReleaseInstallResult | null = null;
  let installLogEntries: InstallLogEntry[] = [];
  let installLogText = "";
  let installRunId = "";
  let installDeviceId: string | null = null;
  let installTargetIp: string | null = null;
  let installDeviceHint: InstallDeviceHint | null = null;
  let installDeviceSnapshot: DiscoveredDevice | null = null;
  let installSteps: UpdaterStep[] = [];
  let installSessionMode: "flash" | "mount" | "ota" | null = null;
  let installCancelBusy = false;
  let installLogContainer: HTMLDivElement | null = null;
  let lastInstallTargetPath: string | null = null;
  let removeUpdaterListener: UnlistenFn | null = null;
  let removeDiscoveryListener: UnlistenFn | null = null;
  let removeTelemetryListener: UnlistenFn | null = null;
  let deviceLogBusy = false;
  let deviceLogInFlight = false;
  let deviceLogError: string | null = null;
  let deviceLogMessage: string | null = null;
  let deviceLogText = "";
  let deviceLogSourceUrl: string | null = null;
  let deviceLogLineCount = 0;
  let deviceLogTruncated = false;
  let deviceLogFetchedAtEpochMs: number | null = null;
  let wpilibLogsBusy = false;
  let wpilibLogsInFlight = false;
  let wpilibLogActionBusy = false;
  let wpilibLogsError: string | null = null;
  let wpilibLogsMessage: string | null = null;
  let wpilibLogsFetchedAtEpochMs: number | null = null;
  let wpilibLogEntries: WpilibLogEntry[] = [];
  let selectedWpilibLogIds = new Set<string>();
  let lastDeviceLogKey = "";
  let currentDeviceLogKey = "";
  let currentTelemetryStreamKey = "";
  let lastDiscoverySignature = "";
  let lastWarningsSignature = "";
  let deviceLastSeenEpochById = new Map<string, number>();
  let isSelectedDeviceUpdateSession = false;
  let installSessionStatus: "running" | "success" | "error" = "running";
  let installLatestError: string | null = null;
  let installProgressFraction = 0;
  let installProgressPercent = 0;
  let installStepProgressPercent: Record<string, number> = {};
  let installStepBytes: Record<string, { written: number | null; total: number | null }> = {};
  let telemetryStreamId = "";
  let telemetryStreamDeviceKey = "";
  let telemetryRestartInFlight = false;
  let telemetryRetryHandle: ReturnType<typeof setTimeout> | null = null;
  let discoveryPollHandle: ReturnType<typeof setInterval> | null = null;
  let telemetryRetryAttempts = 0;
  let telemetryStreamStatus: "idle" | "connecting" | "live" | "error" = "idle";
  let telemetryStreamMessage: string | null = null;
  let telemetrySourceUrl: string | null = null;
  let lastTelemetryEpochMs: number | null = null;
  let liveTelemetrySummary: string | null = null;
  let liveRuntimeProduct: string | null = null;
  let liveOsVersion: string | null = null;
  let powerCycleReminder: PowerCycleReminder | null = null;
  let otaAutoAttachProbeInFlight = false;
  let otaAutoAttachLastProbeEpochMs = 0;
  let otaAutoAttachLastSignature = "";
  const TELEMETRY_RETRY_BASE_MS = 1500;
  const TELEMETRY_RETRY_MAX_MS = 12000;
  const DISCOVERY_REQUEST_TIMEOUT_MS = 9000;
  const OTA_AUTO_ATTACH_PROBE_INTERVAL_MS = 2500;
  // Keep stale endpoints only briefly so role transitions (online -> bootloader)
  // do not show both identities for multiple discovery cycles.
  const DISCOVERY_STALE_REMOVAL_MS = 2500;
  const DISCOVERY_AUTO_REFRESH_MS = 3000;
  const POWER_CYCLE_REMINDER_STORAGE_KEY = "atlas-hardware-manager.power-cycle-reminder.v1";

  $: {
    const discovered = devices.find((device) => device.id === selectedDeviceId) ?? null;
    if (discovered) {
      selectedDevice = discovered;
    } else if (
      installSteps.length > 0 &&
      installDeviceSnapshot &&
      (!selectedDeviceId ||
        selectedDeviceId === installDeviceId ||
        selectedDeviceId === installDeviceSnapshot.id)
    ) {
      selectedDevice = installDeviceSnapshot;
    } else {
      selectedDevice = null;
    }
  }
  $: isInstallStateSelected =
    selectedDevice?.status === "bootloader" || selectedDevice?.status === "mounted";
  $: updateActionLabel = isInstallStateSelected ? "Install OS" : "Update";
  $: updateModalTitle = isInstallStateSelected ? "Install OS Image" : "Update Device Image";
  $: isSelectedDeviceUpdateSession =
    installSteps.length > 0 &&
    !!selectedDevice &&
    (installDeviceId
      ? installDeviceId === selectedDevice.id || installDeviceSnapshot?.id === selectedDevice.id
      : true);
  $: if (installBusy && installTargetIp) {
    const matchedDevice = devices.find((device) => (device.ipAddress ?? "").trim() === installTargetIp);
    if (matchedDevice && installDeviceId !== matchedDevice.id) {
      installDeviceId = matchedDevice.id;
      if (!selectedDeviceId) {
        selectedDeviceId = matchedDevice.id;
      }
    }
  }
  $: installProgressFraction = calculateInstallProgress(installSteps, installStepProgressPercent);
  $: installProgressPercent = Math.round(installProgressFraction * 100);
  $: localImageFileName = localImagePath.split(/[/\\]/).pop() || localImagePath || "";
  $: if (typeof window !== "undefined") {
    try {
      if (powerCycleReminder) {
        localStorage.setItem(POWER_CYCLE_REMINDER_STORAGE_KEY, JSON.stringify(powerCycleReminder));
      } else {
        localStorage.removeItem(POWER_CYCLE_REMINDER_STORAGE_KEY);
      }
    } catch {
      // Ignore localStorage failures and keep in-memory reminder only.
    }
  }
  $: hasSelectedInstallImage =
    imageSource === "release"
      ? selectedReleaseUrl.trim().length > 0
      : localImagePath.trim().length > 0;
  $: canApplyOtaViaApi = !installBusy && hasSelectedInstallImage && !!selectedDevice?.ipAddress?.trim();
  $: canApplyOtaViaUsb = !installBusy && hasSelectedInstallImage;
  $: canApplyInstall =
    !installBusy &&
    (installAction === "mount"
      ? true
      : installAction === "ota"
        ? canApplyOtaViaApi || canApplyOtaViaUsb
        : hasSelectedInstallImage);
  $: if (installAction !== "ota" && otaApplyMenuOpen) {
    otaApplyMenuOpen = false;
  }
  $: if (installBusy && otaApplyMenuOpen) {
    otaApplyMenuOpen = false;
  }
  $: installLogText = installLogEntries
    .map((entry) => {
      const label = entry.level === "error" ? "ERROR" : entry.level === "warn" ? "WARN" : "INFO";
      const timestamp = new Date(entry.timestamp).toLocaleTimeString();
      return `[${timestamp}] [${label}] ${entry.text}`;
    })
    .join("\n");
  $: currentDeviceLogKey = selectedDevice
    ? `${selectedDevice.id}:${selectedDevice.ipAddress ?? ""}:${selectedDevice.runtimeProduct ?? ""}:${selectedDevice.status}`
    : "";
  $: if (currentDeviceLogKey !== lastDeviceLogKey) {
    lastDeviceLogKey = currentDeviceLogKey;
    resetDeviceLogState();
    resetWpilibLogState();
    if (currentDeviceLogKey) {
      if (supportsWpilibLogs(selectedDevice)) {
        void loadWpilibLogs();
      } else {
        void loadDeviceLog();
      }
    }
  }
  $: currentTelemetryStreamKey = selectedDevice
    ? `${selectedDevice.id}:${selectedDevice.ipAddress ?? ""}:${selectedDevice.runtimeProduct ?? ""}:${selectedDevice.status}`
    : "";
  $: if (currentTelemetryStreamKey !== telemetryStreamDeviceKey) {
    telemetryStreamDeviceKey = currentTelemetryStreamKey;
    resetLiveTelemetryState();
    void restartTelemetryStreamForSelectedDevice();
  }
  $: if (
    selectedDevice &&
    isLikelyHeliosDevice(selectedDevice) &&
    releaseOptions.length === 0 &&
    !releasesLoading &&
    !releasesError
  ) {
    void loadReleaseOptions();
  }
  $: if (showUpdateModal && !supportsUpdateAction(selectedDevice)) {
    showUpdateModal = false;
  }
  onMount(() => {
    try {
      const raw = localStorage.getItem(POWER_CYCLE_REMINDER_STORAGE_KEY);
      if (raw) {
        const parsed = JSON.parse(raw) as Partial<PowerCycleReminder>;
        if (
          parsed &&
          typeof parsed.message === "string" &&
          typeof parsed.createdAtEpochMs === "number"
        ) {
          powerCycleReminder = {
            runId: typeof parsed.runId === "string" ? parsed.runId : null,
            deviceId: typeof parsed.deviceId === "string" ? parsed.deviceId : null,
            message: parsed.message,
            createdAtEpochMs: parsed.createdAtEpochMs,
          };
        }
      }
    } catch {
      powerCycleReminder = null;
    }

    setPageHeader({
      title: "NETWORK DEVICES",
      subtitle: "Discover HeliOS devices over USB IP, network IP, and USB bootloader mode.",
      actions: [],
    });

    const refreshDiscoveryIfVisible = () => {
      if (document.visibilityState !== "visible") {
        return;
      }
      void discoverDevices({ background: true });
    };
    const handleWindowFocus = () => {
      refreshDiscoveryIfVisible();
    };
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        refreshDiscoveryIfVisible();
      }
    };

    discoveryPollHandle = setInterval(() => {
      refreshDiscoveryIfVisible();
    }, DISCOVERY_AUTO_REFRESH_MS);
    window.addEventListener("focus", handleWindowFocus);
    document.addEventListener("visibilitychange", handleVisibilityChange);

    void discoverDevices();
    void listen<DeviceDiscoveryProgressEvent>("network-discovery-progress", (event) => {
      const payload = event.payload;
      if (!payload) {
        return;
      }
      const nextDevices = payload.devices ?? [];
      const nextWarnings = payload.warnings ?? [];
      const observedAt = payload.generatedAtEpochMs ?? Date.now();
      const nextWarningsSignature = nextWarnings.join("|");

      mergeDiscoveredDevices(nextDevices, observedAt, { allowPrune: false });
      if (nextWarningsSignature !== lastWarningsSignature) {
        warnings = nextWarnings;
        lastWarningsSignature = nextWarningsSignature;
      }
      lastDiscoveryEpochMs = observedAt;
      ensureSelectedDeviceSelection();
      discoveryError = null;
    }).then((unlisten) => {
      removeDiscoveryListener = unlisten;
    });

    void listen<UpdaterProgressEvent>("updater-progress", (event) => {
      const payload = event.payload;
      if (!payload) {
        return;
      }
      if (!installRunId) {
        return;
      }

      const expectedRunId = installRunId.trim();
      const payloadRunId = payload.runId?.trim() ?? "";
      const activeMode = currentInstallMode();
      if (payloadRunId) {
        if (payloadRunId !== expectedRunId) {
          if (!installBusy || payload.mode !== activeMode) {
            return;
          }
          // Keep receiving progress when backend run-id formatting differs during the same active mode.
          installRunId = payloadRunId;
        }
      } else if (!installBusy || payload.mode !== activeMode) {
        return;
      }

      const normalizedStepStatus: UpdaterProgressEvent["status"] =
        payload.status === "info" &&
        installSteps.some((step) => step.key === payload.step && step.status === "pending")
          ? "running"
          : payload.status;

      applyStepProgress(payload.step, normalizedStepStatus);
      appendInstallLog(
        payload.status === "error" ? "error" : payload.status === "skipped" ? "warn" : "info",
        payload.message,
      );
      if (payload.imagePath) {
        appendInstallLog("info", `Image path: ${payload.imagePath}`);
      }
      if (payload.targetPath && payload.targetPath !== lastInstallTargetPath) {
        lastInstallTargetPath = payload.targetPath;
        appendInstallLog("info", `Target path: ${payload.targetPath}`);
      }
      if (
        payload.progressPercent != null &&
        Number.isFinite(payload.progressPercent) &&
        stepSupportsProgress(payload.step)
      ) {
        installStepProgressPercent = {
          ...installStepProgressPercent,
          [payload.step]: clampPercent(payload.progressPercent),
        };
      }
      if (
        stepSupportsProgress(payload.step) &&
        (payload.bytesWritten != null || payload.bytesTotal != null)
      ) {
        installStepBytes = {
          ...installStepBytes,
          [payload.step]: {
            written: payload.bytesWritten ?? null,
            total: payload.bytesTotal ?? null,
          },
        };
      }
      if (payload.status === "success" && stepSupportsProgress(payload.step)) {
        installStepProgressPercent = {
          ...installStepProgressPercent,
          [payload.step]: 100,
        };
      }
      if (normalizedStepStatus === "running") {
        installSessionStatus = "running";
      } else if (normalizedStepStatus === "error") {
        installSessionStatus = "error";
        recordInstallError(payload.message);
      }
      if (payload.stdout) {
        appendInstallLog("info", payload.stdout);
      }
      if (payload.stderr) {
        appendInstallLog(payload.status === "error" ? "error" : "warn", payload.stderr);
      }
      if (payload.step === "finalize-target" && (payload.status === "success" || payload.status === "skipped")) {
        const normalizedMessage = payload.message.toLowerCase();
        const manualPowerCycleRequired =
          payload.status === "skipped" ||
          normalizedMessage.includes("manually power cycle");
        if (manualPowerCycleRequired) {
          powerCycleReminder = {
            runId: installRunId || null,
            deviceId: installDeviceId ?? null,
            createdAtEpochMs: Date.now(),
            message:
              payload.message ||
              "Flash media ejected. Power cycle the device now before expecting it online.",
          };
        }
      }

      if (payload.step === "complete") {
        if (normalizedStepStatus === "success") {
          installSessionStatus = "success";
        } else {
          installSessionStatus = "error";
        }
        const wasBusy = installBusy;
        installBusy = false;
        installCancelBusy = false;
        if (wasBusy) {
          void discoverDevices({ background: true });
        }
      }
    }).then((unlisten) => {
      removeUpdaterListener = unlisten;
      void recoverOtaUpdateSession();
    });
    void listen<DeviceTelemetryEvent>("device-telemetry", (event) => {
      const payload = event.payload;
      if (!payload || !telemetryStreamId || payload.streamId !== telemetryStreamId) {
        return;
      }

      lastTelemetryEpochMs = payload.timestampEpochMs || Date.now();
      telemetryStreamMessage = payload.message ?? null;
      telemetrySourceUrl = payload.sourceUrl ?? telemetrySourceUrl;
      const telemetrySummary = payload.telemetrySummary?.trim();
      if (payload.runtimeProduct) {
        liveRuntimeProduct = payload.runtimeProduct;
      }
      if (payload.osVersion) {
        liveOsVersion = payload.osVersion;
      }
      if (telemetrySummary) {
        liveTelemetrySummary = telemetrySummary;
      }

      if (payload.status === "connecting") {
        telemetryStreamStatus = "connecting";
        clearTelemetryRetry();
      } else if (payload.status === "data") {
        if (telemetrySummary) {
          telemetryStreamStatus = "live";
          telemetryRetryAttempts = 0;
          clearTelemetryRetry();
        } else if (telemetryStreamStatus !== "live") {
          telemetryStreamStatus = "connecting";
          telemetryStreamMessage = "Connected, waiting for telemetry values.";
        }
      } else if (payload.status === "error") {
        telemetryStreamStatus = "error";
        scheduleTelemetryRetry(payload.message ?? undefined);
      } else if (payload.status === "stopped") {
        telemetryStreamStatus = "idle";
        scheduleTelemetryRetry(payload.message ?? undefined);
      }
    }).then((unlisten) => {
      removeTelemetryListener = unlisten;
    });

    return () => {
      if (removeUpdaterListener) {
        removeUpdaterListener();
        removeUpdaterListener = null;
      }
      if (removeDiscoveryListener) {
        removeDiscoveryListener();
        removeDiscoveryListener = null;
      }
      if (removeTelemetryListener) {
        removeTelemetryListener();
        removeTelemetryListener = null;
      }
      if (discoveryPollHandle) {
        clearInterval(discoveryPollHandle);
        discoveryPollHandle = null;
      }
      window.removeEventListener("focus", handleWindowFocus);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      void stopTelemetryStream();
      resetPageHeader();
    };
  });

  async function discoverDevices(options: { background?: boolean } = {}): Promise<void> {
    const background = options.background ?? false;
    if (discoveryInFlight) {
      if (!background) {
        discoveryManualQueued = true;
        discoveryBusy = true;
        discoveryError = null;
      }
      return;
    }

    const requestGeneration = discoveryRequestGeneration;
    discoveryInFlight = true;
    if (!background) {
      discoveryBusy = true;
      discoveryError = null;
    }

    try {
      const snapshot = await withTimeout(
        invoke<DeviceDiscoverySnapshot>("discover_network_devices"),
        DISCOVERY_REQUEST_TIMEOUT_MS,
        "Device discovery timed out. Retry refresh.",
      );
      if (requestGeneration !== discoveryRequestGeneration) {
        return;
      }
      const observedAt = snapshot.generatedAtEpochMs ?? Date.now();
      const nextWarningsSignature = snapshot.warnings.join("|");
      const hasWarningChanges = nextWarningsSignature !== lastWarningsSignature;

      mergeDiscoveredDevices(snapshot.devices, observedAt, { allowPrune: true });
      if (!background || hasWarningChanges) {
        warnings = snapshot.warnings;
        lastWarningsSignature = nextWarningsSignature;
      }
      lastDiscoveryEpochMs = observedAt;
      ensureSelectedDeviceSelection();
      if (devices.length > 0) {
        rebindInstallDevice(devices);
      }
      lastDiscoveryEpochMs = snapshot.generatedAtEpochMs;

      if (!selectedDeviceId || !devices.some((device) => device.id === selectedDeviceId)) {
        if (installSteps.length > 0 && installDeviceId) {
          selectedDeviceId = installDeviceId;
        } else {
          selectedDeviceId = devices[0]?.id ?? null;
        }
      }
      void maybeAutoAttachExistingOtaUpdate();
    } catch (error) {
      if (requestGeneration !== discoveryRequestGeneration) {
        return;
      }
      if (!background) {
        discoveryError = error instanceof Error ? error.message : "Device discovery failed.";
      }
    } finally {
      if (requestGeneration === discoveryRequestGeneration) {
        discoveryInFlight = false;
        if (!background) {
          discoveryBusy = false;
        }
        if (discoveryManualQueued) {
          discoveryManualQueued = false;
          void discoverDevices();
        }
      }
    }
  }

  function withTimeout<T>(promise: Promise<T>, timeoutMs: number, timeoutMessage: string): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const timeoutHandle = setTimeout(() => {
        reject(new Error(timeoutMessage));
      }, timeoutMs);
      promise
        .then((value) => {
          clearTimeout(timeoutHandle);
          resolve(value);
        })
        .catch((error: unknown) => {
          clearTimeout(timeoutHandle);
          reject(error);
        });
    });
  }

  function cancelDiscovery(): void {
    discoveryRequestGeneration += 1;
    discoveryManualQueued = false;
    discoveryInFlight = false;
    discoveryBusy = false;
    discoveryError = "Discovery canceled.";
  }

  function refreshInstallDeviceHint(device: DiscoveredDevice | null): void {
    if (!device) {
      return;
    }
    installDeviceHint = {
      usbLocation: device.usbLocation ?? null,
      macAddress: device.macAddress ?? null,
      ipAddress: device.ipAddress ?? null,
      vendorProduct: device.vendorProduct ?? null,
      runtimeProduct: device.runtimeProduct ?? null,
      displayName: device.displayName ?? null,
    };
  }

  function rebindInstallDevice(nextDevices: DiscoveredDevice[]): void {
    if (!installBusy || !installDeviceId || nextDevices.length === 0) {
      return;
    }

    const active = nextDevices.find((device) => device.id === installDeviceId);
    if (active) {
      installDeviceSnapshot = { ...active };
      refreshInstallDeviceHint(active);
      return;
    }

    const hint = installDeviceHint;
    let fallback: DiscoveredDevice | null = null;
    if (installTargetIp) {
      fallback = nextDevices.find((device) => (device.ipAddress ?? "").trim() === installTargetIp) ?? null;
    }
    if (!fallback && hint) {
      const scored = nextDevices
        .map((device) => {
          let score = 0;
          if (hint.usbLocation && device.usbLocation && hint.usbLocation === device.usbLocation) score += 6;
          if (hint.macAddress && device.macAddress && hint.macAddress === device.macAddress) score += 5;
          if (hint.ipAddress && device.ipAddress && hint.ipAddress === device.ipAddress) score += 4;
          if (hint.vendorProduct && device.vendorProduct && hint.vendorProduct === device.vendorProduct) score += 2;
          if (hint.runtimeProduct && device.runtimeProduct && hint.runtimeProduct === device.runtimeProduct) score += 2;
          if (hint.displayName && device.displayName && hint.displayName === device.displayName) score += 1;
          return { device, score };
        })
        .filter((entry) => entry.score > 0)
        .sort((a, b) => b.score - a.score);
      if (scored.length > 0) {
        const best = scored[0];
        const sameScoreCount = scored.filter((entry) => entry.score === best.score).length;
        if (sameScoreCount === 1) {
          fallback = best.device;
        }
      }
    }
    if (!fallback) {
      const usbCandidates = nextDevices.filter(
        (device) => device.status === "bootloader" || device.status === "mounted",
      );
      if (usbCandidates.length === 1) {
        fallback = usbCandidates[0];
      }
    }
    if (!fallback && nextDevices.length === 1) {
      fallback = nextDevices[0];
    }
    if (!fallback) {
      return;
    }

    const previousInstallId = installDeviceId;
    installDeviceId = fallback.id;
    installDeviceSnapshot = { ...fallback };
    refreshInstallDeviceHint(fallback);
    if (
      !selectedDeviceId ||
      selectedDeviceId === previousInstallId ||
      !nextDevices.some((device) => device.id === selectedDeviceId)
    ) {
      selectedDeviceId = fallback.id;
    }
  }

  async function recoverOtaUpdateSession(): Promise<void> {
    if (installBusy || installRunId) {
      return;
    }

    try {
      const recovered = await invoke<UpdaterRecoverySession | null>("recover_ota_update_session");
      if (!recovered || recovered.mode !== "ota" || !recovered.runId) {
        return;
      }

      installBusy = true;
      installCancelBusy = false;
      installError = null;
      installResult = null;
      powerCycleReminder = null;
      installRunId = recovered.runId;
      installTargetIp = recovered.targetIpAddress?.trim() || null;
      installSessionMode = "ota";
      installDeviceId =
        installTargetIp
          ? (devices.find((device) => (device.ipAddress ?? "").trim() === installTargetIp)?.id ?? null)
          : null;
      installDeviceSnapshot =
        installDeviceId != null ? (devices.find((device) => device.id === installDeviceId) ?? null) : null;
      refreshInstallDeviceHint(installDeviceSnapshot);
      if (installDeviceId && selectedDeviceId !== installDeviceId) {
        selectedDeviceId = installDeviceId;
      }
      installSteps = buildStepTemplate("ota");
      installStepProgressPercent = {};
      installStepBytes = {};
      lastInstallTargetPath = null;
      installSessionStatus = "running";
      installLatestError = null;

      beginInstallLog("ota");
      appendInstallLog("warn", recovered.note?.trim() || "Recovered OTA update session after restart.");
      if (installTargetIp) {
        appendInstallLog("info", `Recovered target IP: ${installTargetIp}.`);
      }
      if (recovered.expectedUpdateId?.trim()) {
        appendInstallLog("info", `Recovered update ID: ${recovered.expectedUpdateId}.`);
      }
    } catch {
      // Ignore recovery failures; normal update flows remain available.
    }
  }

  function otaAutoAttachCandidates(): DiscoveredDevice[] {
    const preferredId = selectedDeviceId;
    const scored = devices
      .filter((device) => {
        if (device.status !== "online") {
          return false;
        }
        const ip = device.ipAddress?.trim() ?? "";
        if (!ip) {
          return false;
        }
        return isLikelyHeliosDevice(device);
      })
      .map((device) => {
        let score = 0;
        if (preferredId && device.id === preferredId) {
          score += 100;
        }
        if (device.telemetrySummary?.trim()) {
          score += 10;
        }
        if (device.runtimeProduct?.trim()) {
          score += 5;
        }
        return { device, score };
      })
      .sort((a, b) => b.score - a.score);

    const uniqueByIp = new Set<string>();
    const candidates: DiscoveredDevice[] = [];
    for (const entry of scored) {
      const ip = entry.device.ipAddress?.trim() ?? "";
      if (!ip || uniqueByIp.has(ip)) {
        continue;
      }
      uniqueByIp.add(ip);
      candidates.push(entry.device);
    }
    return candidates;
  }

  function normalizeOtaStageForUi(value: string | null | undefined): string {
    const normalized = (value ?? "").trim().toLowerCase();
    if (!normalized) {
      return "unknown";
    }
    return normalized.replace(/\s+/g, "_");
  }

  function otaStageProgressHintForUi(stage: string): number | null {
    if (stage === "downloading") return 15;
    if (stage === "verifying") return 35;
    if (stage === "awaiting_window") return 55;
    if (stage === "applying" || stage === "installing") return 75;
    if (stage === "committing") return 88;
    if (stage === "finalizing") return 94;
    if (stage === "pending_reboot" || stage === "pending-reboot") return 97;
    if (stage === "rebooting" || stage === "complete") return 100;
    return null;
  }

  function seedOtaAttachSessionFromProbe(probe: ExistingOtaUpdateProbeResult): void {
    const stage = normalizeOtaStageForUi(probe.stage);
    const hasExplicitProgress =
      probe.progressPercent != null && Number.isFinite(probe.progressPercent);
    const progress = hasExplicitProgress
      ? clampPercent(probe.progressPercent as number)
      : otaStageProgressHintForUi(stage);

    applyStepProgress("resolve-image", "success");
    applyStepProgress("resolve-target", "success");
    applyStepProgress("ota-upload", "success");

    const rebootLikeStage =
      stage === "pending_reboot" || stage === "pending-reboot" || stage === "rebooting" || stage === "complete";
    if (rebootLikeStage) {
      applyStepProgress("ota-apply", "success");
      applyStepProgress("ota-monitor", "success");
      applyStepProgress("ota-reconnect", "running");
      installStepProgressPercent = {
        ...installStepProgressPercent,
        "ota-apply": 100,
        "ota-monitor": 100,
        "ota-reconnect": 40,
      };
      return;
    }

    applyStepProgress("ota-apply", "running");
    applyStepProgress("ota-monitor", "running");
    if (progress != null) {
      installStepProgressPercent = {
        ...installStepProgressPercent,
        "ota-apply": progress,
        "ota-monitor": progress,
      };
    }
  }

  function generateInstallRunId(): string {
    return typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
      ? crypto.randomUUID()
      : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  }

  async function attachToExistingOtaUpdateSession(
    probe: ExistingOtaUpdateProbeResult,
  ): Promise<boolean> {
    const targetIp = probe.targetIpAddress?.trim() ?? "";
    if (!targetIp) {
      return false;
    }

    const runId = generateInstallRunId();
    const matchedDevice =
      devices.find((device) => (device.ipAddress ?? "").trim() === targetIp) ?? null;

    installBusy = true;
    installCancelBusy = false;
    installError = null;
    installResult = null;
    powerCycleReminder = null;
    installRunId = runId;
    installTargetIp = targetIp;
    installSessionMode = "ota";
    installDeviceId = matchedDevice?.id ?? null;
    installDeviceSnapshot = matchedDevice ? { ...matchedDevice } : null;
    refreshInstallDeviceHint(matchedDevice);
    if (installDeviceId && selectedDeviceId !== installDeviceId) {
      selectedDeviceId = installDeviceId;
    }
    installSteps = buildStepTemplate("ota");
    installStepProgressPercent = {};
    installStepBytes = {};
    lastInstallTargetPath = null;
    installSessionStatus = "running";
    installLatestError = null;

    beginInstallLog("ota");
    appendInstallLog(
      "warn",
      `Detected an in-progress OTA update on ${targetIp}. Attaching to live OTA monitor.`,
    );
    if (probe.updateId?.trim()) {
      appendInstallLog("info", `Detected update ID: ${probe.updateId.trim()}.`);
    }
    if (probe.stage?.trim()) {
      const progress = probe.progressPercent != null && Number.isFinite(probe.progressPercent)
        ? ` at ${Math.round(clampPercent(probe.progressPercent))}%`
        : "";
      appendInstallLog("info", `Detected OTA stage: ${probe.stage.trim()}${progress}.`);
    }
    if (probe.lastError?.trim()) {
      appendInstallLog("warn", `Device reported last OTA error: ${probe.lastError.trim()}`);
    }
    seedOtaAttachSessionFromProbe(probe);

    try {
      const started = await invoke<string>("attach_existing_ota_update", {
        request: {
          runId,
          targetIpAddress: targetIp,
          expectedUpdateId: probe.updateId?.trim() || null,
          timeoutSeconds: 900,
        },
      });
      appendInstallLog("info", started);
      return true;
    } catch (error) {
      installError =
        error instanceof Error ? error.message : "Unable to attach to existing OTA update.";
      recordInstallError(installError);
      appendInstallLog("error", installError);
      installBusy = false;
      installCancelBusy = false;
      installSessionStatus = "error";
      return false;
    }
  }

  async function maybeAutoAttachExistingOtaUpdate(): Promise<void> {
    if (otaAutoAttachProbeInFlight) {
      return;
    }
    if (installBusy || installRunId || installSteps.length > 0) {
      return;
    }
    const now = Date.now();
    if (now - otaAutoAttachLastProbeEpochMs < OTA_AUTO_ATTACH_PROBE_INTERVAL_MS) {
      return;
    }

    const candidates = otaAutoAttachCandidates();
    if (candidates.length === 0) {
      return;
    }

    otaAutoAttachProbeInFlight = true;
    otaAutoAttachLastProbeEpochMs = now;
    try {
      const probe = await invoke<ExistingOtaUpdateProbeResult | null>(
        "probe_existing_device_ota_update",
        {
          request: {
            targetIpAddresses: candidates.map((device) => device.ipAddress?.trim() ?? ""),
            timeoutMs: 1500,
          },
        },
      );
      if (!probe || !probe.targetIpAddress?.trim()) {
        return;
      }

      const targetIp = probe.targetIpAddress.trim();
      const updateId = probe.updateId?.trim() ?? "";
      const stage = normalizeOtaStageForUi(probe.stage);
      const progressBucket =
        probe.progressPercent != null && Number.isFinite(probe.progressPercent)
          ? Math.round(clampPercent(probe.progressPercent) / 10)
          : -1;
      const signature = updateId
        ? `${targetIp}|${updateId}`
        : `${targetIp}|${stage}|${progressBucket}`;
      if (signature === otaAutoAttachLastSignature) {
        return;
      }

      const attached = await attachToExistingOtaUpdateSession(probe);
      if (attached) {
        otaAutoAttachLastSignature = signature;
      }
    } catch {
      // Ignore auto-attach probe failures; manual update actions remain available.
    } finally {
      otaAutoAttachProbeInFlight = false;
    }
  }

  function dismissPowerCycleReminder(): void {
    powerCycleReminder = null;
  }

  function powerCycleReminderLabel(): string {
    if (!powerCycleReminder) {
      return "";
    }
    return `Set ${new Date(powerCycleReminder.createdAtEpochMs).toLocaleTimeString()}`;
  }

  function buildDevicesSignature(list: DiscoveredDevice[]): string {
    return list
      .map((device) =>
        [
          device.id,
          device.status,
          device.runtimeProduct ?? "",
          device.firmwareVersion ?? "",
          device.osVersion ?? "",
          device.telemetrySummary ?? "",
          device.ipAddress ?? "",
          device.macAddress ?? "",
          device.connectionChips.join(","),
        ].join("~"),
      )
      .join("|");
  }

  function ensureSelectedDeviceSelection(): void {
    if (selectedDeviceId && devices.some((device) => device.id === selectedDeviceId)) {
      return;
    }
    if (installSteps.length > 0 && installDeviceId) {
      selectedDeviceId = installDeviceId;
      return;
    }
    selectedDeviceId = devices[0]?.id ?? null;
  }

  function mergeDiscoveredDevices(
    incoming: DiscoveredDevice[],
    observedAtEpochMs: number,
    options: { allowPrune: boolean },
  ): void {
    const stabilizedIncoming = incoming.map((device) => stabilizeIncomingDeviceIdentity(device));
    const nextById = new Set(stabilizedIncoming.map((device) => device.id));
    const incomingIdentityKeys = new Set<string>();
    for (const device of stabilizedIncoming) {
      for (const alias of deviceIdentityAliases(device)) {
        incomingIdentityKeys.add(alias);
      }
    }
    for (const device of stabilizedIncoming) {
      deviceLastSeenEpochById.set(device.id, observedAtEpochMs);
    }

    const retained = devices.filter((device) => {
      if (nextById.has(device.id)) {
        return false;
      }
      const identityAliases = deviceIdentityAliases(device);
      if (identityAliases.some((alias) => incomingIdentityKeys.has(alias))) {
        return false;
      }
      if (!options.allowPrune) {
        return true;
      }
      const lastSeen = deviceLastSeenEpochById.get(device.id) ?? observedAtEpochMs;
      return observedAtEpochMs - lastSeen < DISCOVERY_STALE_REMOVAL_MS;
    });

    const merged = dedupeDevicesByIdentity([...stabilizedIncoming, ...retained]);
    const mergedSignature = buildDevicesSignature(merged);
    if (mergedSignature !== lastDiscoverySignature) {
      devices = merged;
      lastDiscoverySignature = mergedSignature;
    }

    if (options.allowPrune) {
      const activeIds = new Set(merged.map((device) => device.id));
      for (const [id, lastSeen] of deviceLastSeenEpochById) {
        if (!activeIds.has(id) && observedAtEpochMs - lastSeen >= DISCOVERY_STALE_REMOVAL_MS) {
          deviceLastSeenEpochById.delete(id);
        }
      }
    }
  }

  function stabilizeIncomingDeviceIdentity(incoming: DiscoveredDevice): DiscoveredDevice {
    const incomingAliases = new Set(deviceIdentityAliases(incoming));
    const previous = devices.find((existing) => {
      if (existing.id === incoming.id) {
        return true;
      }
      return deviceIdentityAliases(existing).some((alias) => incomingAliases.has(alias));
    });
    if (!previous) {
      return incoming;
    }
    if (!isRoboRioDevice(previous) || isRoboRioDevice(incoming)) {
      return incoming;
    }

    const mergedChips = [...incoming.connectionChips];
    for (const chip of previous.connectionChips) {
      if (!mergedChips.includes(chip)) {
        mergedChips.push(chip);
      }
    }
    const normalizedName = incoming.displayName.trim().toLowerCase();
    const hasWeakName =
      !normalizedName || normalizedName === "unknown device" || normalizedName === "network device";

    return {
      ...incoming,
      displayName: hasWeakName ? "roboRIO device" : incoming.displayName,
      runtimeProduct: incoming.runtimeProduct ?? previous.runtimeProduct,
      connectionChips: mergedChips,
    };
  }

  function dedupeDevicesByIdentity(list: DiscoveredDevice[]): DiscoveredDevice[] {
    const deduped: Array<DiscoveredDevice | null> = [];
    const indexByAlias = new Map<string, number>();
    for (const device of list) {
      const aliases = deviceIdentityAliases(device);
      const matchedIndexes = new Set<number>();
      for (const alias of aliases) {
        const existingIndex = indexByAlias.get(alias);
        if (existingIndex != null && deduped[existingIndex]) {
          matchedIndexes.add(existingIndex);
        }
      }

      if (matchedIndexes.size === 0) {
        const newIndex = deduped.length;
        deduped.push(device);
        for (const alias of aliases) {
          indexByAlias.set(alias, newIndex);
        }
        continue;
      }

      let keeperIndex: number | null = null;
      for (const index of matchedIndexes) {
        if (keeperIndex == null) {
          keeperIndex = index;
          continue;
        }
        const candidate = deduped[index];
        const keeper = deduped[keeperIndex];
        if (candidate && keeper && deviceIdentityScore(candidate) > deviceIdentityScore(keeper)) {
          keeperIndex = index;
        }
      }
      if (keeperIndex == null) {
        continue;
      }

      let keeperDevice = deduped[keeperIndex];
      if (!keeperDevice || deviceIdentityScore(device) > deviceIdentityScore(keeperDevice)) {
        deduped[keeperIndex] = device;
        keeperDevice = device;
      }

      for (const index of matchedIndexes) {
        if (index === keeperIndex) {
          continue;
        }
        const mergedOut = deduped[index];
        if (!mergedOut) {
          continue;
        }
        for (const alias of deviceIdentityAliases(mergedOut)) {
          indexByAlias.set(alias, keeperIndex);
        }
        deduped[index] = null;
      }

      for (const alias of aliases) {
        indexByAlias.set(alias, keeperIndex);
      }
      if (keeperDevice) {
        for (const alias of deviceIdentityAliases(keeperDevice)) {
          indexByAlias.set(alias, keeperIndex);
        }
      }
    }

    return deduped.filter((device): device is DiscoveredDevice => !!device);
  }

  function deviceIdentityAliases(device: DiscoveredDevice): string[] {
    const aliases: string[] = [];
    const ip = normalizeIdentityComponent(device.ipAddress);
    const mac = normalizeIdentityComponent(device.macAddress);
    const iface = normalizeIdentityComponent(device.interfaceName);
    const usb = normalizeIdentityComponent(device.usbLocation);

    if (ip) {
      aliases.push(`ip:${ip}`);
    }
    if (mac) {
      aliases.push(`mac:${mac}`);
    }
    if (ip && mac) {
      aliases.push(`ipmac:${ip}|${mac}`);
    }
    if (ip && iface) {
      aliases.push(`ipif:${ip}|${iface}`);
    }
    if (mac && iface) {
      aliases.push(`macif:${mac}|${iface}`);
    }
    if (usb && ip) {
      aliases.push(`usbip:${usb}|${ip}`);
    }
    if (usb && mac) {
      aliases.push(`usbmac:${usb}|${mac}`);
    }

    if (aliases.length === 0) {
      const vendor = normalizeIdentityComponent(device.vendorProduct);
      if (usb) {
        aliases.push(`usb:${usb}`);
      }
      if (iface && vendor) {
        aliases.push(`ifvendor:${iface}|${vendor}`);
      }
      aliases.push(`id:${device.id}`);
    }

    return aliases;
  }

  function normalizeIdentityComponent(value?: string | null): string {
    const normalized = (value ?? "").trim().toLowerCase();
    return normalized.length ? normalized : "";
  }

  function deviceIdentityScore(device: DiscoveredDevice): number {
    const product = (device.runtimeProduct ?? "").toLowerCase();
    const displayName = (device.displayName ?? "").toLowerCase();
    let score = 0;

    if (device.status === "online") {
      score += 120;
    } else if (device.status === "bootloader" || device.status === "mounted") {
      score += 80;
    }
    if (isRoboRioDevice(device)) {
      score += 90;
    }
    if (
      product.includes("helios") ||
      product.includes("prometheus") ||
      product.includes("photonvision") ||
      product.includes("limelight")
    ) {
      score += 50;
    }
    if (device.telemetrySummary?.trim()) {
      score += 16;
    }
    if (device.firmwareVersion?.trim()) {
      score += 8;
    }
    if (device.osVersion?.trim()) {
      score += 8;
    }
    if (displayName.includes("roborio")) {
      score += 10;
    }
    if (displayName.includes("network device")) {
      score -= 20;
    }

    return score;
  }

  function resetDeviceLogState(): void {
    deviceLogError = null;
    deviceLogMessage = null;
    deviceLogText = "";
    deviceLogSourceUrl = null;
    deviceLogLineCount = 0;
    deviceLogTruncated = false;
    deviceLogFetchedAtEpochMs = null;
  }

  function resetWpilibLogState(): void {
    wpilibLogsError = null;
    wpilibLogsMessage = null;
    wpilibLogsFetchedAtEpochMs = null;
    wpilibLogEntries = [];
    selectedWpilibLogIds = new Set<string>();
  }

  function isRoboRioDevice(device: DiscoveredDevice | null): boolean {
    if (!device) {
      return false;
    }
    const runtimeProduct = (device.runtimeProduct ?? "").toLowerCase();
    if (runtimeProduct.includes("roborio") || runtimeProduct === "rio") {
      return true;
    }
    if (device.displayName.toLowerCase().includes("roborio")) {
      return true;
    }
    return device.connectionChips.some((chip) => chip.toLowerCase().includes("roborio"));
  }

  function supportsWpilibLogs(device: DiscoveredDevice | null): boolean {
    if (!device?.ipAddress || device.status !== "online") {
      return false;
    }
    return isRoboRioDevice(device);
  }

  function supportsDeviceLog(device: DiscoveredDevice | null): boolean {
    if (!device?.ipAddress) {
      return false;
    }
    if (device.status !== "online") {
      return false;
    }
    if (supportsWpilibLogs(device)) {
      return false;
    }
    const product = (device.runtimeProduct ?? "").toLowerCase();
    return product.includes("photonvision") || product.includes("helios") || product.includes("prometheus");
  }

  function supportsWebUiAction(device: DiscoveredDevice | null): boolean {
    if (!device?.ipAddress || device.status !== "online") {
      return false;
    }
    return !isRoboRioDevice(device);
  }

  function supportsUpdateAction(device: DiscoveredDevice | null): boolean {
    if (!device) {
      return false;
    }
    return !isRoboRioDevice(device);
  }

  async function loadDeviceLog(options: { background?: boolean } = {}): Promise<void> {
    if (deviceLogInFlight) {
      return;
    }

    const background = options.background ?? false;
    const device = selectedDevice;
    if (!supportsDeviceLog(device)) {
      if (!background) {
        resetDeviceLogState();
      }
      return;
    }

    const requestKey = currentDeviceLogKey;
    const ipAddress = device?.ipAddress ?? "";
    const runtimeProduct = device?.runtimeProduct ?? null;
    deviceLogInFlight = true;
    if (!background) {
      deviceLogBusy = true;
      deviceLogError = null;
    }

    try {
      const result = await invoke<DeviceLogResult>("fetch_device_log", {
        request: {
          ipAddress,
          runtimeProduct,
          maxLines: 300,
        },
      });

      if (requestKey !== currentDeviceLogKey) {
        return;
      }

      if (!result.success) {
        if (deviceLogText) {
          deviceLogText = "";
        }
        if (deviceLogSourceUrl) {
          deviceLogSourceUrl = null;
        }
        if (deviceLogLineCount !== 0) {
          deviceLogLineCount = 0;
        }
        if (deviceLogTruncated) {
          deviceLogTruncated = false;
        }
        deviceLogFetchedAtEpochMs = result.fetchedAtEpochMs;
        deviceLogMessage = result.message || "No logs available.";
        return;
      }

      const nextLogText = result.logText ?? "";
      const nextSourceUrl = result.sourceUrl ?? null;
      const nextLineCount = result.lineCount ?? 0;
      const nextTruncated = result.truncated ?? false;
      if (
        deviceLogText !== nextLogText ||
        deviceLogSourceUrl !== nextSourceUrl ||
        deviceLogLineCount !== nextLineCount ||
        deviceLogTruncated !== nextTruncated
      ) {
        deviceLogText = nextLogText;
        deviceLogSourceUrl = nextSourceUrl;
        deviceLogLineCount = nextLineCount;
        deviceLogTruncated = nextTruncated;
      }
      deviceLogFetchedAtEpochMs = result.fetchedAtEpochMs ?? Date.now();
      deviceLogMessage = result.message ?? null;
      deviceLogError = null;
    } catch (error) {
      if (requestKey !== currentDeviceLogKey) {
        return;
      }
      if (!background) {
        resetDeviceLogState();
      }
      deviceLogError = error instanceof Error ? error.message : "Failed to load device logs.";
    } finally {
      if (requestKey === currentDeviceLogKey) {
        deviceLogBusy = false;
      }
      deviceLogInFlight = false;
    }
  }

  async function loadWpilibLogs(options: { background?: boolean } = {}): Promise<void> {
    if (wpilibLogsInFlight) {
      return;
    }

    const background = options.background ?? false;
    const device = selectedDevice;
    if (!supportsWpilibLogs(device)) {
      if (!background) {
        resetWpilibLogState();
      }
      return;
    }

    const requestKey = currentDeviceLogKey;
    wpilibLogsInFlight = true;
    if (!background) {
      wpilibLogsBusy = true;
      wpilibLogsError = null;
    }

    try {
      const result = await invoke<WpilibLogListResult>("list_roborio_wpilib_logs", {
        request: {
          ipAddress: device?.ipAddress ?? "",
        },
      });
      if (requestKey !== currentDeviceLogKey) {
        return;
      }

      wpilibLogEntries = result.entries ?? [];
      wpilibLogsMessage = result.message ?? null;
      wpilibLogsFetchedAtEpochMs = result.fetchedAtEpochMs ?? Date.now();
      wpilibLogsError = null;
      const validIds = new Set(wpilibLogEntries.map((entry) => entry.id));
      selectedWpilibLogIds = new Set(
        Array.from(selectedWpilibLogIds).filter((id) => validIds.has(id)),
      );
    } catch (error) {
      if (requestKey !== currentDeviceLogKey) {
        return;
      }
      if (!background) {
        resetWpilibLogState();
      }
      wpilibLogsError =
        error instanceof Error ? error.message : "Failed to list WPILib logs from roboRIO.";
    } finally {
      if (requestKey === currentDeviceLogKey) {
        wpilibLogsBusy = false;
      }
      wpilibLogsInFlight = false;
    }
  }

  function toggleWpilibLogSelection(logId: string): void {
    const next = new Set(selectedWpilibLogIds);
    if (next.has(logId)) {
      next.delete(logId);
    } else {
      next.add(logId);
    }
    selectedWpilibLogIds = next;
  }

  function selectAllWpilibLogs(): void {
    selectedWpilibLogIds = new Set(wpilibLogEntries.map((entry) => entry.id));
  }

  function clearWpilibLogSelection(): void {
    selectedWpilibLogIds = new Set<string>();
  }

  function selectedWpilibLogs(): WpilibLogSelection[] {
    const selectedIds = selectedWpilibLogIds;
    return wpilibLogEntries
      .filter((entry) => selectedIds.has(entry.id))
      .map((entry) => ({
        fileName: entry.fileName,
        remotePath: entry.remotePath,
      }));
  }

  async function runWpilibLogAction(
    action: "download" | "delete" | "downloadDelete",
  ): Promise<void> {
    if (wpilibLogActionBusy) {
      return;
    }
    const device = selectedDevice;
    if (!supportsWpilibLogs(device)) {
      return;
    }
    const selected = selectedWpilibLogs();
    if (selected.length === 0) {
      wpilibLogsError = "Select at least one WPILib log file.";
      return;
    }

    let downloadDirectory: string | null = null;
    if (action !== "delete") {
      const selectedDir = await openFileDialog({
        title: "Choose WPILib log download folder",
        multiple: false,
        directory: true,
      });
      if (!selectedDir) {
        return;
      }
      downloadDirectory = Array.isArray(selectedDir) ? selectedDir[0] ?? null : selectedDir;
      if (!downloadDirectory) {
        return;
      }
    }

    wpilibLogActionBusy = true;
    wpilibLogsError = null;
    try {
      const commandName =
        action === "download"
          ? "download_roborio_wpilib_logs"
          : action === "delete"
            ? "delete_roborio_wpilib_logs"
            : "download_and_delete_roborio_wpilib_logs";
      const result = await invoke<WpilibLogActionResult>(commandName, {
        request: {
          ipAddress: device?.ipAddress ?? "",
          selectedLogs: selected,
          downloadDirectory: downloadDirectory ?? null,
        },
      });
      wpilibLogsFetchedAtEpochMs = result.completedAtEpochMs ?? Date.now();
      wpilibLogsMessage = result.message ?? null;
      const failures = (result.results ?? []).filter((item) => !item.success);
      wpilibLogsError =
        failures.length > 0
          ? failures.map((item) => `${item.fileName}: ${item.message}`).join("\n")
          : null;

      if (action !== "download") {
        const deletedPaths = new Set(
          (result.results ?? []).filter((item) => item.success).map((item) => item.remotePath),
        );
        wpilibLogEntries = wpilibLogEntries.filter((entry) => !deletedPaths.has(entry.remotePath));
        selectedWpilibLogIds = new Set(
          Array.from(selectedWpilibLogIds).filter(
            (id) => wpilibLogEntries.find((entry) => entry.id === id) != null,
          ),
        );
      }

      void loadWpilibLogs({ background: true });
    } catch (error) {
      wpilibLogsError =
        error instanceof Error ? error.message : "WPILib log action failed for roboRIO.";
    } finally {
      wpilibLogActionBusy = false;
    }
  }

  function resetLiveTelemetryState(): void {
    clearTelemetryRetry();
    telemetryRetryAttempts = 0;
    telemetryStreamStatus = "idle";
    telemetryStreamMessage = null;
    telemetrySourceUrl = null;
    lastTelemetryEpochMs = null;
    liveTelemetrySummary = null;
    liveRuntimeProduct = null;
    liveOsVersion = null;
  }

  function clearTelemetryRetry(): void {
    if (!telemetryRetryHandle) {
      return;
    }
    clearTimeout(telemetryRetryHandle);
    telemetryRetryHandle = null;
  }

  function scheduleTelemetryRetry(reason?: string): void {
    const device = selectedDevice;
    if (!supportsTelemetryStream(device) || telemetryRestartInFlight || telemetryRetryHandle) {
      return;
    }

    const attempt = telemetryRetryAttempts + 1;
    telemetryRetryAttempts = attempt;
    const delayMs = Math.min(
      TELEMETRY_RETRY_BASE_MS * 2 ** Math.min(attempt - 1, 3),
      TELEMETRY_RETRY_MAX_MS,
    );

    const reasonText = reason?.trim();
    telemetryStreamMessage = reasonText?.length
      ? `${reasonText} Retrying in ${(delayMs / 1000).toFixed(1)}s (attempt ${attempt}).`
      : `Retrying telemetry connection in ${(delayMs / 1000).toFixed(1)}s (attempt ${attempt}).`;

    telemetryRetryHandle = setTimeout(() => {
      telemetryRetryHandle = null;
      void restartTelemetryStreamForSelectedDevice();
    }, delayMs);
  }

  function supportsTelemetryStream(device: DiscoveredDevice | null): boolean {
    if (!device?.ipAddress || device.status !== "online") {
      return false;
    }
    const product = (device.runtimeProduct ?? "").toLowerCase();
    if (isRoboRioDevice(device)) {
      return true;
    }
    return product.includes("photonvision") || product.includes("helios") || product.includes("prometheus");
  }

  async function stopTelemetryStream(): Promise<void> {
    clearTelemetryRetry();
    const streamId = telemetryStreamId;
    telemetryStreamId = "";
    if (!streamId) {
      return;
    }
    try {
      await invoke<string>("stop_device_telemetry_stream", {
        request: {
          streamId,
        },
      });
    } catch {
      // Stream stop errors are non-fatal when switching devices.
    }
  }

  async function restartTelemetryStreamForSelectedDevice(): Promise<void> {
    if (telemetryRestartInFlight) {
      return;
    }
    clearTelemetryRetry();
    telemetryRestartInFlight = true;
    try {
      await stopTelemetryStream();
      const device = selectedDevice;
      if (!supportsTelemetryStream(device)) {
        return;
      }
      if (!device?.ipAddress) {
        return;
      }
      telemetryStreamStatus = "connecting";
      telemetryStreamMessage = "Connecting telemetry websocket.";
      telemetryStreamId =
        typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
          ? crypto.randomUUID()
          : `${Date.now()}-${Math.random().toString(16).slice(2)}`;

      await invoke<string>("start_device_telemetry_stream", {
        request: {
          streamId: telemetryStreamId,
          ipAddress: device.ipAddress,
          runtimeProduct: device.runtimeProduct ?? null,
        },
      });
    } catch (error) {
      telemetryStreamStatus = "error";
      telemetryStreamMessage =
        error instanceof Error ? error.message : "Unable to start telemetry websocket stream.";
      scheduleTelemetryRetry(telemetryStreamMessage ?? undefined);
    } finally {
      telemetryRestartInFlight = false;
    }
  }

  function runtimeProductForDevice(device: DiscoveredDevice): string {
    if (selectedDevice?.id === device.id && liveRuntimeProduct) {
      return liveRuntimeProduct;
    }
    return device.runtimeProduct ?? "";
  }

  function osVersionForDevice(device: DiscoveredDevice): string {
    if (selectedDevice?.id === device.id && liveOsVersion) {
      return liveOsVersion;
    }
    return device.osVersion ?? "";
  }

  function telemetrySummaryForDevice(device: DiscoveredDevice): string {
    if (selectedDevice?.id === device.id && liveTelemetrySummary) {
      return liveTelemetrySummary;
    }
    if (selectedDevice?.id === device.id && supportsTelemetryStream(device) && !isRoboRioDevice(device)) {
      return "";
    }
    return device.telemetrySummary ?? "";
  }

  async function openSelectedWebUi(): Promise<void> {
    actionError = null;
    if (!supportsWebUiAction(selectedDevice) || !selectedDevice?.ipAddress) {
      return;
    }

    try {
      await openUrl(`http://${selectedDevice.ipAddress}:5800/`);
    } catch (error) {
      actionError = error instanceof Error ? error.message : "Unable to open WebUI.";
    }
  }

  async function loadReleaseOptions(): Promise<void> {
    releasesLoading = true;
    releasesError = null;

    try {
      releaseOptions = await invoke<ReleaseImageOption[]>("list_helios_release_images");
      if (!selectedReleaseUrl || !releaseOptions.some((option) => option.downloadUrl === selectedReleaseUrl)) {
        selectedReleaseUrl = releaseOptions[0]?.downloadUrl ?? "";
      }
    } catch (error) {
      releasesError =
        error instanceof Error ? error.message : "Unable to load release images from GitHub.";
      releaseOptions = [];
      selectedReleaseUrl = "";
    } finally {
      releasesLoading = false;
    }
  }

  async function openUpdateModal(): Promise<void> {
    if (!supportsUpdateAction(selectedDevice)) {
      actionError = "Updates are not available for this device.";
      return;
    }
    showUpdateModal = true;
    otaApplyMenuOpen = false;
    installError = null;
    installResult = null;
    actionError = null;
    installAction =
      selectedDevice?.status === "bootloader"
        ? "mount"
        : selectedDevice?.status === "mounted"
          ? "flash"
          : "ota";
    if (releaseOptions.length === 0 && (installAction === "flash" || installAction === "ota")) {
      await loadReleaseOptions();
    }
  }

  function closeUpdateModal(): void {
    otaApplyMenuOpen = false;
    showUpdateModal = false;
  }

  function handleModalBackdropClick(event: MouseEvent): void {
    if (event.target === event.currentTarget) {
      closeUpdateModal();
    }
  }

  async function pickLocalImageFile(): Promise<void> {
    installError = null;

    try {
      const selected = await openFileDialog({
        title: "Select HeliOS image",
        multiple: false,
        directory: false,
        filters: [
          {
            name: "HeliOS Images",
            extensions: ["img", "xz", "wic", "zip", "iso", "upd"],
          },
        ],
      });

      if (!selected) {
        return;
      }

      localImagePath = Array.isArray(selected) ? selected[0] ?? "" : selected;
    } catch (error) {
      installError = error instanceof Error ? error.message : "Unable to open file picker.";
    }
  }

  async function applyImageInstall(): Promise<void> {
    if (!canApplyInstall) {
      return;
    }

    installBusy = true;
    installCancelBusy = false;
    installError = null;
    installResult = null;
    startInstallSession("flash");
    beginInstallLog("flash");
    showUpdateModal = false;
    appendInstallLog("info", "Running install flow: rpiboot -> rediscover target -> flash image.");
    appendInstallLog(
      "info",
      imageSource === "release"
        ? `Image source: GitHub release (${selectedReleaseUrl || "not selected"}).`
        : `Image source: Local image (${localImagePath || "not selected"}).`,
    );

    try {
      appendInstallLog("info", "Starting background install workflow.");
      const started = await invoke<string>("start_install_helios_os", {
        request: {
          timeoutSeconds: 90,
          runId: installRunId,
          preferOta: false,
          targetIpAddress: null,
          selectedBootloaderId: selectedBootloaderIdForRequest(),
          releaseDownloadUrl: imageSource === "release" ? selectedReleaseUrl : null,
          localImagePath: imageSource === "local" ? localImagePath.trim() : null,
        },
      });
      appendInstallLog("info", started);
    } catch (error) {
      installError =
        error instanceof Error ? error.message : "Install flow failed to start.";
      recordInstallError(installError);
      appendInstallLog("error", installError);
      installBusy = false;
      installCancelBusy = false;
      installSessionStatus = "error";
    }
  }

  async function applyOtaInstall(transport: OtaApplyTransport = "api"): Promise<void> {
    if (transport === "api" && !canApplyOtaViaApi) {
      return;
    }
    if (transport === "usb" && !canApplyOtaViaUsb) {
      return;
    }
    const targetIp = transport === "api" ? selectedDevice?.ipAddress?.trim() ?? "" : "";
    if (transport === "api" && !targetIp) {
      installError = "Selected device does not have an IP address for OTA.";
      recordInstallError(installError);
      return;
    }

    otaApplyMenuOpen = false;
    installBusy = true;
    installCancelBusy = false;
    installError = null;
    installResult = null;
    startInstallSession("ota");
    installTargetIp = targetIp || null;
    beginInstallLog("ota");
    showUpdateModal = false;
    appendInstallLog(
      "info",
      transport === "usb"
        ? "Running OTA flow over USB recovery protocol: resolve transport -> upload -> activate."
        : "Running OTA flow over API: resolve image -> upload -> apply -> monitor reboot -> wait for reconnect.",
    );
    if (targetIp) {
      appendInstallLog("info", `OTA target IP: ${targetIp}.`);
    } else {
      appendInstallLog("info", "OTA transport: USB recovery serial protocol.");
    }
    appendInstallLog(
      "info",
      imageSource === "release"
        ? `Image source: GitHub release (${selectedReleaseUrl || "not selected"}).`
        : `Image source: Local image (${localImagePath || "not selected"}).`,
    );

    try {
      appendInstallLog("info", "Starting background OTA workflow.");
      const started = await invoke<string>("start_install_helios_os", {
        request: {
          timeoutSeconds: 600,
          runId: installRunId,
          preferOta: true,
          targetIpAddress: transport === "api" ? targetIp : null,
          selectedBootloaderId: null,
          releaseDownloadUrl: imageSource === "release" ? selectedReleaseUrl : null,
          localImagePath: imageSource === "local" ? localImagePath.trim() : null,
        },
      });
      appendInstallLog("info", started);
    } catch (error) {
      installError = error instanceof Error ? error.message : "OTA workflow failed to start.";
      recordInstallError(installError);
      appendInstallLog("error", installError);
      installBusy = false;
      installCancelBusy = false;
      installSessionStatus = "error";
    }
  }

  async function applyMountOnly(): Promise<void> {
    if (!isInstallStateSelected || installBusy) {
      return;
    }

    installBusy = true;
    installCancelBusy = false;
    installError = null;
    installResult = null;
    startInstallSession("mount");
    beginInstallLog("mount");
    showUpdateModal = false;
    appendInstallLog("info", "Running mount flow: rpiboot -> rediscover target (no image write).");

    try {
      appendInstallLog("info", "Starting background mount workflow.");
      const started = await invoke<string>("start_mount_helios_bootloader", {
        request: {
          timeoutSeconds: 90,
          selectedBootloaderId: selectedBootloaderIdForRequest(),
          runId: installRunId,
        },
      });
      appendInstallLog("info", started);
    } catch (error) {
      installError = error instanceof Error ? error.message : "Mount flow failed to start.";
      recordInstallError(installError);
      appendInstallLog("error", installError);
      installBusy = false;
      installCancelBusy = false;
      installSessionStatus = "error";
    }
  }

  function selectedBootloaderIdForRequest(): string | null {
    const device = selectedDevice;
    if (!device) {
      return null;
    }
    if (device.status === "bootloader" || device.status === "mounted") {
      return device.id;
    }
    return null;
  }

  async function applySelectedInstallAction(): Promise<void> {
    if (installAction === "mount") {
      await applyMountOnly();
      return;
    }
    if (installAction === "ota") {
      await applyOtaInstall("api");
      return;
    }

    await applyImageInstall();
  }

  function toggleOtaApplyMenu(event: MouseEvent): void {
    event.stopPropagation();
    if (!canApplyOtaViaApi && !canApplyOtaViaUsb) {
      return;
    }
    otaApplyMenuOpen = !otaApplyMenuOpen;
  }

  function chooseOtaApplyTransport(transport: OtaApplyTransport): void {
    otaApplyMenuOpen = false;
    void applyOtaInstall(transport);
  }

  async function cancelActiveUpdate(): Promise<void> {
    if (!installBusy || !installRunId || installCancelBusy) {
      return;
    }

    installCancelBusy = true;
    installError = null;
    appendInstallLog("warn", "Cancel requested by user. Waiting for current operation to stop.");

    try {
      const cancelMessage = await invoke<string>("cancel_helios_update", {
        request: {
          runId: installRunId,
          mode: currentInstallMode(),
        },
      });
      appendInstallLog("warn", cancelMessage);
    } catch (error) {
      const message = error instanceof Error ? error.message : "Unable to cancel updater.";
      installError = message;
      recordInstallError(message);
      appendInstallLog("error", message);
    } finally {
      installCancelBusy = false;
    }
  }

  function startInstallSession(mode: "flash" | "mount" | "ota"): void {
    installRunId = generateInstallRunId();
    installDeviceId = selectedDeviceId;
    installDeviceSnapshot = selectedDevice ? { ...selectedDevice } : null;
    refreshInstallDeviceHint(selectedDevice);
    installTargetIp = null;
    installSessionMode = mode;
    powerCycleReminder = null;
    installSteps = buildStepTemplate(mode);
    installStepProgressPercent = {};
    installStepBytes = {};
    lastInstallTargetPath = null;
    installSessionStatus = "running";
    installLatestError = null;
  }

  function closeInstallSessionPanel(): void {
    if (installBusy) {
      return;
    }
    installRunId = "";
    installDeviceId = null;
    installDeviceHint = null;
    installDeviceSnapshot = null;
    installTargetIp = null;
    installSessionMode = null;
    installSteps = [];
    installStepProgressPercent = {};
    installStepBytes = {};
    installLogEntries = [];
    installError = null;
    installResult = null;
    installCancelBusy = false;
    lastInstallTargetPath = null;
    installSessionStatus = "running";
    installLatestError = null;
  }

  function buildStepTemplate(mode: "flash" | "mount" | "ota"): UpdaterStep[] {
    if (mode === "ota") {
      return [
        { key: "resolve-image", label: "Resolve Image", detail: "Download/select update image.", status: "pending" },
        { key: "resolve-target", label: "Resolve Target", detail: "Resolve OTA API endpoint for the selected device.", status: "pending" },
        { key: "ota-upload", label: "Upload OTA", detail: "Upload image to OTA endpoint.", status: "pending" },
        { key: "ota-apply", label: "Apply OTA", detail: "Trigger and track device-side OTA apply stages.", status: "pending" },
        { key: "ota-monitor", label: "Track OTA State", detail: "Read OTA stage/progress from device state.", status: "pending" },
        { key: "ota-reconnect", label: "Wait Reconnect", detail: "Wait for reboot and online reconnect.", status: "pending" },
        { key: "complete", label: "Complete", detail: "OTA workflow finished.", status: "pending" },
      ];
    }
    if (mode === "mount") {
      return [
        { key: "scan-targets", label: "Scan Targets", detail: "Inspect removable flash targets.", status: "pending" },
        { key: "bootloader-check", label: "Detect Bootloader", detail: "Check USB bootloader presence.", status: "pending" },
        { key: "rpiboot", label: "Run rpiboot", detail: "Mount mass-storage gadget if needed.", status: "pending" },
        { key: "select-target", label: "Select Target", detail: "Choose mounted flash disk.", status: "pending" },
        { key: "complete", label: "Complete", detail: "Mount workflow finished.", status: "pending" },
      ];
    }

    return [
      { key: "resolve-image", label: "Resolve Image", detail: "Download/select install image.", status: "pending" },
      { key: "scan-targets", label: "Scan Targets", detail: "Inspect removable flash targets.", status: "pending" },
      { key: "bootloader-check", label: "Detect Bootloader", detail: "Check USB bootloader presence.", status: "pending" },
      { key: "rpiboot", label: "Run rpiboot", detail: "Mount mass-storage gadget if needed.", status: "pending" },
      { key: "select-target", label: "Select Target", detail: "Choose flash disk for write.", status: "pending" },
      { key: "flash", label: "Write Image", detail: "Flash image to selected target.", status: "pending" },
      { key: "verify", label: "Verify Image", detail: "Verify flashed target against source image.", status: "pending" },
      { key: "finalize-target", label: "Finalize Device", detail: "Attempt eject/power-cycle after write.", status: "pending" },
      { key: "complete", label: "Complete", detail: "Install workflow finished.", status: "pending" },
    ];
  }

  function applyStepProgress(step: string, status: UpdaterProgressEvent["status"]): void {
    const stepIndex = installSteps.findIndex((entry) => entry.key === step);
    if (stepIndex < 0) {
      return;
    }

    const nextSteps = [...installSteps];

    if (status === "running") {
      if (
        nextSteps[stepIndex].status !== "done" &&
        nextSteps[stepIndex].status !== "error" &&
        nextSteps[stepIndex].status !== "skipped"
      ) {
        nextSteps[stepIndex] = { ...nextSteps[stepIndex], status: "active" };
      }
      installSteps = nextSteps;
      return;
    }

    if (status === "success") {
      nextSteps[stepIndex] = { ...nextSteps[stepIndex], status: "done" };
      installSteps = nextSteps;
      return;
    }

    if (status === "skipped") {
      nextSteps[stepIndex] = { ...nextSteps[stepIndex], status: "skipped" };
      installSteps = nextSteps;
      return;
    }

    if (status === "error") {
      nextSteps[stepIndex] = { ...nextSteps[stepIndex], status: "error" };
      installSteps = nextSteps;
    }
  }

  function beginInstallLog(mode: "flash" | "mount" | "ota"): void {
    installLogEntries = [];
    appendInstallLog("info", `Updater session started (${mode.toUpperCase()}).`);
  }

  function recordInstallError(message: string | null | undefined): void {
    const text = message?.trim();
    if (!text) {
      return;
    }
    installLatestError = text;
  }

  function appendInstallLog(level: "info" | "warn" | "error", text: string): void {
    const lines = text
      .split(/\r?\n/)
      .map((line) => line.trimEnd())
      .filter((line) => line.length > 0);

    if (lines.length === 0) {
      return;
    }
    if (level === "error") {
      recordInstallError(lines[lines.length - 1]);
    }

    const nextEntries = lines.map((line) => ({
      timestamp: Date.now(),
      level,
      text: line,
    }));
    installLogEntries = [...installLogEntries, ...nextEntries];

    queueMicrotask(() => {
      if (installLogContainer) {
        installLogContainer.scrollTop = installLogContainer.scrollHeight;
      }
    });
  }

  function calculateInstallProgress(
    steps: UpdaterStep[],
    stepProgressPercent: Record<string, number>,
  ): number {
    if (steps.length === 0) {
      return 0;
    }

    const otaMode = steps.some((step) => step.key.startsWith("ota-"));
    const flashMode = !otaMode && steps.some((step) => step.key === "flash");
    const weightByStep = new Map<string, number>(
      otaMode
        ? [
            ["resolve-image", 0.05],
            ["resolve-target", 0.05],
            ["ota-upload", 0.2],
            ["ota-apply", 0.1],
            ["ota-monitor", 0.35],
            ["ota-reconnect", 0.25],
            ["complete", 0.0],
          ]
        : flashMode
        ? [
            ["resolve-image", 0.03],
            ["scan-targets", 0.03],
            ["bootloader-check", 0.03],
            ["rpiboot", 0.03],
            ["select-target", 0.03],
            ["flash", 0.7],
            ["verify", 0.1],
            ["finalize-target", 0.05],
            ["complete", 0.0],
          ]
        : [
            ["scan-targets", 0.25],
            ["bootloader-check", 0.2],
            ["rpiboot", 0.2],
            ["select-target", 0.3],
            ["complete", 0.05],
          ],
    );

    let completedWeight = 0;
    let totalWeight = 0;
    for (const step of steps) {
      const weight = weightByStep.get(step.key) ?? 0;
      totalWeight += weight;

      if (weight <= 0) {
        continue;
      }

      if (step.status === "done" || step.status === "skipped" || step.status === "error") {
        completedWeight += weight;
      } else if (step.status === "active") {
        const progress = stepProgressPercent[step.key];
        if (progress != null && Number.isFinite(progress)) {
          completedWeight += weight * (clampPercent(progress) / 100);
        } else {
          completedWeight += weight * 0.5;
        }
      }
    }

    if (totalWeight <= 0) {
      return 0;
    }

    return Math.min(1, Math.max(0, completedWeight / totalWeight));
  }

  function currentInstallMode(): "flash" | "mount" | "ota" {
    if (installSessionMode) {
      return installSessionMode;
    }
    if (installSteps.some((step) => step.key.startsWith("ota-"))) {
      return "ota";
    }
    return installSteps.some((step) => step.key === "flash") ? "flash" : "mount";
  }

  function activeInstallStepLabel(): string {
    const active = installSteps.find((step) => step.status === "active");
    if (active) {
      return active.label;
    }

    const failed = installSteps.find((step) => step.status === "error");
    if (failed) {
      return `${failed.label} failed`;
    }

    const pending = installSteps.find((step) => step.status === "pending");
    if (pending) {
      return pending.label;
    }

    return installSteps.length > 0 ? "Finishing" : "Starting";
  }

  function installSessionStateClass(): string {
    if (installBusy || installSessionStatus === "running") {
      return "border-primary-500/50 bg-primary-500/10 text-primary-100";
    }
    if (installSessionStatus === "success") {
      return "border-success-500/50 bg-success-500/10 text-success-100";
    }
    return "border-error-500/50 bg-error-500/10 text-error-100";
  }

  function installSessionStateLabel(): string {
    if (installBusy || installSessionStatus === "running") {
      return `Running · ${activeInstallStepLabel()}`;
    }
    if (installSessionStatus === "success") {
      return "Completed";
    }
    return "Failed";
  }

  function formatBytes(value: number): string {
    if (!value || value <= 0) {
      return "0 B";
    }
    const units = ["B", "KB", "MB", "GB", "TB"];
    const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
    const scaled = value / 1024 ** index;
    return `${scaled.toFixed(scaled >= 10 ? 0 : 1)} ${units[index]}`;
  }

  function clampPercent(value: number): number {
    return Math.min(100, Math.max(0, value));
  }

  function stepSupportsProgress(stepKey: string): boolean {
    return stepKey === "flash" || stepKey === "verify" || stepKey.startsWith("ota-");
  }

  function installStepProgressValue(step: UpdaterStep): number | null {
    if (!stepSupportsProgress(step.key) || step.status === "skipped") {
      return null;
    }
    if (step.status === "done") {
      return 100;
    }
    const progress = installStepProgressPercent[step.key];
    if (progress == null || !Number.isFinite(progress)) {
      return step.status === "active" ? 0 : null;
    }
    return clampPercent(progress);
  }

  function installStepProgressLabel(step: UpdaterStep): string | null {
    const progress = installStepProgressValue(step);
    if (progress == null) {
      return null;
    }
    const bytes = installStepBytes[step.key];
    if (bytes?.written != null && bytes.total != null && bytes.total > 0) {
      return `${Math.round(progress)}% · ${formatBytes(bytes.written)} / ${formatBytes(bytes.total)}`;
    }
    return `${Math.round(progress)}%`;
  }

  function chipClass(chip: string): string {
    if (chip === "PhotonVision") {
      return "border-tertiary-500/50 bg-tertiary-500/10 text-tertiary-100";
    }
    if (chip === "HeliOS") {
      return "border-success-500/50 bg-success-500/10 text-success-100";
    }
    if (chip === "Bootloader Device") {
      return "border-warning-500/50 bg-warning-500/10 text-warning-100";
    }
    if (chip === "Mounted") {
      return "border-success-500/50 bg-success-500/10 text-success-100";
    }
    if (chip === "USB IP") {
      return "border-primary-500/50 bg-primary-500/10 text-primary-100";
    }
    return "border-secondary-500/50 bg-secondary-500/10 text-secondary-100";
  }

  function statusClass(status: string): string {
    if (status === "bootloader") {
      return "text-warning-300";
    }
    if (status === "mounted") {
      return "text-success-300";
    }
    return "text-success-300";
  }

  function statusLabel(status: string): string {
    if (status === "bootloader") {
      return "Bootloader";
    }
    if (status === "mounted") {
      return "Mounted";
    }
    return "Online";
  }

  function panelConnectionChips(device: DiscoveredDevice): string[] {
    return device.connectionChips.filter((chip) => {
      if (device.status === "bootloader" && chip === "Bootloader Device") {
        return false;
      }
      if (device.status === "mounted" && chip === "Mounted") {
        return false;
      }
      return true;
    });
  }

  function compactFacts(facts: DeviceFact[]): DeviceFact[] {
    const seen = new Set<string>();
    return facts.filter((fact) => {
      const value = fact.value.trim();
      if (!value) {
        return false;
      }
      const normalized = `${fact.label.toLowerCase()}::${value.toLowerCase()}`;
      if (seen.has(normalized)) {
        return false;
      }
      seen.add(normalized);
      return true;
    });
  }

  function networkFacts(device: DiscoveredDevice): DeviceFact[] {
    const facts: DeviceFact[] = [
      { label: "Status", value: statusLabel(device.status) },
      { label: "IP", value: device.ipAddress ?? "" },
      { label: "MAC", value: device.macAddress ?? "" },
      { label: "Interface", value: device.interfaceName ?? "" },
      { label: "USB", value: device.usbLocation ?? "" },
      { label: "Vendor/Product", value: device.vendorProduct ?? "" },
      { label: "ID", value: device.id },
    ];

    return compactFacts(facts);
  }

  function softwareFacts(device: DiscoveredDevice): DeviceFact[] {
    return compactFacts([
      { label: "Product", value: runtimeProductForDevice(device) },
      { label: "OS", value: osVersionForDevice(device) },
    ]);
  }

  function isLikelyHeliosDevice(device: DiscoveredDevice | null): boolean {
    if (!device) {
      return false;
    }
    const product = (device.runtimeProduct ?? "").toLowerCase();
    if (product.includes("helios") || product.includes("prometheus")) {
      return true;
    }
    return device.connectionChips.some((chip) => {
      const normalized = chip.toLowerCase();
      return (
        normalized.includes("helios") ||
        normalized.includes("bootloader") ||
        normalized.includes("mounted")
      );
    });
  }

  function telemetryFacts(device: DiscoveredDevice): DeviceFact[] {
    const segments = telemetrySummaryForDevice(device)
      .split("·")
      .map((segment) => segment.trim())
      .filter((segment) => segment.length > 0);
    const facts = segments.map((segment, index) => {
      const separator = segment.indexOf(" ");
      if (separator > 0) {
        const label = segment.slice(0, separator).trim();
        const value = segment.slice(separator + 1).trim();
        if (label && value) {
          return { label, value };
        }
      }
      return { label: `Metric ${index + 1}`, value: segment };
    });

    return compactFacts(facts);
  }

  function telemetryUpdatedLabel(): string {
    const timestamp = lastTelemetryEpochMs ?? lastDiscoveryEpochMs;
    if (!timestamp) {
      return "Waiting for data";
    }
    const deltaMs = Date.now() - timestamp;
    if (deltaMs < 1000) {
      return "Updated just now";
    }
    const seconds = Math.floor(deltaMs / 1000);
    return `Updated ${seconds}s ago`;
  }

  function deviceLogUpdatedLabel(): string {
    if (!deviceLogFetchedAtEpochMs) {
      return "No log fetch yet";
    }
    const seconds = Math.max(0, Math.floor((Date.now() - deviceLogFetchedAtEpochMs) / 1000));
    if (seconds < 1) {
      return "Updated just now";
    }
    return `Updated ${seconds}s ago`;
  }

  function wpilibLogsUpdatedLabel(): string {
    if (!wpilibLogsFetchedAtEpochMs) {
      return "No log fetch yet";
    }
    const seconds = Math.max(0, Math.floor((Date.now() - wpilibLogsFetchedAtEpochMs) / 1000));
    if (seconds < 1) {
      return "Updated just now";
    }
    return `Updated ${seconds}s ago`;
  }

  function installStepClass(status: UpdaterStepStatus): string {
    if (status === "done") {
      return "border-success-500/40 bg-success-500/10 text-success-100";
    }
    if (status === "active") {
      return "border-primary-500/50 bg-primary-500/10 text-primary-100";
    }
    if (status === "error") {
      return "border-error-500/50 bg-error-500/10 text-error-100";
    }
    if (status === "skipped") {
      return "border-warning-500/40 bg-warning-500/10 text-warning-100";
    }
    return "border-surface-700/70 bg-surface-900/60 text-surface-300";
  }

  function installStepStateLabel(status: UpdaterStepStatus): string {
    if (status === "done") return "Done";
    if (status === "active") return "Running";
    if (status === "error") return "Error";
    if (status === "skipped") return "Skipped";
    return "Pending";
  }

  function hasFailedInstallForDevice(deviceId: string): boolean {
    return !installBusy && installSessionStatus === "error" && installDeviceId === deviceId;
  }

</script>

<svelte:window
  onclick={() => {
    otaApplyMenuOpen = false;
  }}
  onkeydown={(event) => {
    if (event.key === "Escape") {
      otaApplyMenuOpen = false;
    }
  }}
/>

<div class="flex h-full min-h-0 min-w-0 flex-1 flex-row items-stretch gap-6 overflow-hidden">
  <aside class="flex h-full min-h-0 w-[26rem] max-w-[34%] min-w-[300px] flex-col overflow-hidden rounded border border-surface-800/60 bg-surface-900/60 shadow-[0_0_40px_-28px_rgba(0,0,0,1)]">
    <div class="border-b border-surface-800/60 p-4">
      <div class="flex items-start justify-between gap-3">
        <div class="flex flex-col">
          <p class="uppercase text-[0.58rem] tracking-[0.32em] text-surface-500">Device Roster</p>
          <h2 class="mt-2 text-lg font-semibold text-surface-50">Discovered Devices</h2>
          <p class="text-xs text-surface-400">USB IP, network IP, and bootloader endpoints.</p>
        </div>
        <div class="flex items-center gap-2">
          <button
            class={`inline-flex h-9 w-9 items-center justify-center rounded border bg-surface-900/80 text-surface-200 transition ${
              discoveryInFlight
                ? "border-error-500/60 hover:border-error-400 hover:text-error-200"
                : "border-surface-700/80 hover:border-primary-500/60 hover:text-primary-200"
            }`}
            type="button"
            aria-label={discoveryInFlight ? "Stop discovery" : "Refresh discovery"}
            title={discoveryInFlight ? "Stop discovery" : "Refresh discovery"}
            onclick={() => {
              if (discoveryInFlight) {
                cancelDiscovery();
                return;
              }
              discoveryError = null;
              void discoverDevices();
            }}
          >
            {#if discoveryInFlight}
              <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <line x1="6" y1="6" x2="18" y2="18"></line>
                <line x1="18" y1="6" x2="6" y2="18"></line>
              </svg>
            {:else}
              <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M21 12a9 9 0 1 1-2.64-6.36"></path>
                <polyline points="21 3 21 9 15 9"></polyline>
              </svg>
            {/if}
          </button>
        </div>
      </div>
    </div>

    <div class="flex-1 divide-y divide-surface-800/60 overflow-y-auto px-2 py-3">
      {#if devices.length === 0 && !discoveryBusy}
        <div class="rounded-md border border-surface-800/70 bg-surface-900/40 px-3 py-4 text-xs text-surface-400">
          No devices discovered.
        </div>
      {/if}

      {#each devices as device (device.id)}
        <button
          class={`flex w-full flex-col gap-2 rounded-md px-3 py-3 text-left transition ${
            hasFailedInstallForDevice(device.id)
              ? "bg-error-500/10 ring-1 ring-inset ring-error-500/35"
              : device.id === selectedDeviceId
              ? "bg-primary-500/10 ring-1 ring-inset ring-primary-500/30"
              : "hover:bg-surface-900/70"
          }`}
          onclick={() => {
            selectedDeviceId = device.id;
          }}
        >
          <div class="flex min-w-0 items-start justify-between gap-3">
            <div class="min-w-0">
              <p class="truncate text-sm font-semibold text-surface-50">{device.displayName}</p>
              <p class="truncate text-[0.65rem] uppercase tracking-[0.25em] text-surface-500">{device.id}</p>
            </div>
            <span class={`text-[0.6rem] uppercase tracking-[0.3em] ${statusClass(device.status)}`}>
              {statusLabel(device.status)}
            </span>
          </div>

          <p class="text-xs text-surface-300">
            {device.ipAddress ?? "No IP"}
            {#if device.macAddress}
              · {device.macAddress}
            {/if}
          </p>

          <div class="flex flex-wrap gap-2">
            {#each device.connectionChips as chip (`${device.id}-${chip}`)}
              <span class={`rounded border px-2 py-1 text-[0.58rem] uppercase tracking-[0.22em] ${chipClass(chip)}`}>
                {chip}
              </span>
            {/each}
          </div>

          {#if installBusy && installDeviceId === device.id}
            <div class="space-y-1">
              <div class="h-1.5 overflow-hidden rounded-full bg-surface-800/80">
                <div
                  class="h-full rounded-full bg-primary-400 transition-all duration-300"
                  style={`width: ${Math.max(6, installProgressPercent)}%`}
                ></div>
              </div>
              <p class="truncate text-[0.62rem] uppercase tracking-[0.16em] text-primary-200">
                Updating · {activeInstallStepLabel()}
              </p>
            </div>
          {:else if hasFailedInstallForDevice(device.id)}
            <p class="truncate text-[0.62rem] uppercase tracking-[0.16em] text-error-200">
              Update failed · {installLatestError ?? "Open updater for details."}
            </p>
          {:else if powerCycleReminder && powerCycleReminder.deviceId === device.id}
            <p class="truncate text-[0.62rem] uppercase tracking-[0.16em] text-warning-200">
              Power cycle required
            </p>
          {/if}
        </button>
      {/each}
    </div>
  </aside>

  <div class="flex h-full min-h-0 min-w-0 w-full flex-col gap-4 overflow-x-hidden overflow-y-auto pr-1">
    {#if discoveryError}
      <div class="rounded-md border border-error-500/40 bg-error-500/10 px-3 py-3 text-sm text-error-100">
        {discoveryError}
      </div>
    {/if}

    {#if actionError}
      <div class="rounded-md border border-error-500/40 bg-error-500/10 px-3 py-3 text-sm text-error-100">
        {actionError}
      </div>
    {/if}

    {#if powerCycleReminder}
      <div class="rounded-md border border-warning-500/45 bg-warning-500/10 px-3 py-3 text-warning-100">
        <div class="flex items-start justify-between gap-3">
          <div>
            <p class="text-[0.6rem] uppercase tracking-[0.22em] text-warning-200">Action Required</p>
            <p class="mt-1 text-sm font-semibold">Power cycle the flashed device</p>
            <p class="mt-1 text-xs text-warning-100">{powerCycleReminder.message}</p>
            <p class="mt-2 text-[0.62rem] uppercase tracking-[0.18em] text-warning-200/90">{powerCycleReminderLabel()}</p>
          </div>
          <button
            class="inline-flex h-8 items-center rounded border border-warning-400/45 bg-warning-500/15 px-3 text-[0.62rem] uppercase tracking-[0.18em] text-warning-100 transition hover:border-warning-300/70 hover:text-warning-50"
            type="button"
            onclick={dismissPowerCycleReminder}
          >
            Dismiss
          </button>
        </div>
      </div>
    {/if}

    {#if warnings.length}
      <div class="rounded-md border border-warning-500/40 bg-warning-500/10 px-3 py-3 text-xs text-warning-100">
        {#each warnings as warning (warning)}
          <p>{warning}</p>
        {/each}
      </div>
    {/if}

    {#if selectedDevice}
      <Panel eyebrow="Selected Device" title={selectedDevice.displayName}>
        <svelte:fragment slot="actions">
          {#if isSelectedDeviceUpdateSession}
            <button
              class={`btn btn-3xs uppercase tracking-[0.2em] disabled:cursor-not-allowed disabled:opacity-60 ${
                installBusy
                  ? "preset-tonal-surface border-warning-500/60 text-warning-100"
                  : "preset-tonal-secondary"
              }`}
              type="button"
              disabled={installBusy ? installCancelBusy : false}
              onclick={() => {
                if (installBusy) {
                  void cancelActiveUpdate();
                  return;
                }
                closeInstallSessionPanel();
              }}
            >
              {#if installBusy}
                {installCancelBusy ? "Canceling..." : "Cancel Update"}
              {:else}
                Close Updater
              {/if}
            </button>
          {:else}
            {#if supportsWebUiAction(selectedDevice)}
              <button
                class="btn btn-3xs preset-tonal-secondary uppercase tracking-[0.2em]"
                type="button"
                onclick={() => void openSelectedWebUi()}
              >
                Open WebUI
              </button>
            {/if}
            {#if supportsUpdateAction(selectedDevice)}
              <button
                class="btn btn-3xs preset-tonal-surface uppercase tracking-[0.2em]"
                type="button"
                onclick={() => void openUpdateModal()}
              >
                {updateActionLabel}
              </button>
            {/if}
          {/if}
        </svelte:fragment>
        <div class="grid min-w-0 gap-5">
          <div class="grid min-w-0 gap-4 lg:grid-cols-2">
            {#if isSelectedDeviceUpdateSession}
              <section class="min-w-0 rounded border border-surface-800/60 bg-surface-900/60 p-4 lg:col-span-2">
                <div class="mb-4 flex items-center justify-between gap-3">
                  <p class="uppercase text-[0.58rem] tracking-[0.32em] text-surface-500">Updater Activity</p>
                  <span class={`rounded border px-2 py-1 text-[0.56rem] uppercase tracking-[0.22em] ${installSessionStateClass()}`}>
                    {installSessionStateLabel()}
                  </span>
                </div>

                <div class="mb-4 h-2 overflow-hidden rounded-full bg-surface-800/80">
                  <div
                    class="h-full rounded-full bg-primary-400 transition-all duration-300"
                    style={`width: ${Math.max(6, installProgressPercent)}%`}
                  ></div>
                </div>

                {#if !installBusy && installSessionStatus === "success"}
                  <div class="mb-4 rounded border border-success-500/45 bg-success-500/10 px-3 py-3 text-sm text-success-100">
                    <p class="text-[0.6rem] uppercase tracking-[0.22em] text-success-200">Update Completed</p>
                    <p class="mt-1">The update finished successfully. Review the log below before closing the updater.</p>
                    {#if currentInstallMode() === "flash"}
                      <p class="mt-2 text-warning-100">Power cycle the device now (or unplug/replug USB) to boot into the updated image.</p>
                    {/if}
                  </div>
                {/if}

                {#if installLatestError}
                  <div class="mb-4 rounded border border-error-500/45 bg-error-500/10 px-3 py-3 text-sm text-error-100">
                    <p class="text-[0.6rem] uppercase tracking-[0.22em] text-error-200">Latest Error</p>
                    <p class="mt-1">{installLatestError}</p>
                  </div>
                {/if}

                <div class="grid min-h-0 items-stretch gap-3 lg:grid-cols-[18rem_minmax(0,1fr)]">
                  <section class="space-y-2 rounded border border-surface-800/70 bg-surface-950/40 p-3">
                    <p class="uppercase text-[0.56rem] tracking-[0.3em] text-surface-500">Workflow Steps</p>
                    <div class="grid gap-2">
                      {#each installSteps as step (step.key)}
                        <div class={`rounded border px-2 py-2 text-xs ${installStepClass(step.status)}`}>
                          <div class="flex items-center justify-between gap-2">
                            <p class="font-semibold uppercase tracking-[0.14em]">{step.label}</p>
                            <span class="text-[0.58rem] uppercase tracking-[0.18em]">{installStepStateLabel(step.status)}</span>
                          </div>
                          <p class="mt-1 text-[0.68rem] opacity-90">{step.detail}</p>
                          {#if installStepProgressValue(step) != null}
                            <div class="mt-2">
                              <div class="h-1.5 overflow-hidden rounded-full bg-surface-800/80">
                                <div
                                  class={`h-full rounded-full transition-all duration-300 ${
                                    step.status === "error"
                                      ? "bg-error-400"
                                      : step.status === "done"
                                        ? "bg-success-400"
                                        : "bg-primary-400"
                                  }`}
                                  style={`width: ${Math.max(6, installStepProgressValue(step) ?? 0)}%`}
                                ></div>
                              </div>
                              {#if installStepProgressLabel(step)}
                                <p class="mt-1 text-[0.56rem] uppercase tracking-[0.14em] text-surface-400">
                                  {installStepProgressLabel(step)}
                                </p>
                              {/if}
                            </div>
                          {/if}
                        </div>
                      {/each}
                    </div>
                  </section>

                  <section class="flex h-full min-h-0 flex-col rounded border border-surface-800/70 bg-surface-950/60">
                    <div class="flex items-center justify-between border-b border-surface-800/70 px-3 py-2">
                      <p class="uppercase text-[0.56rem] tracking-[0.3em] text-surface-500">Live Command Log</p>
                      <span class="text-[0.62rem] uppercase tracking-[0.2em] text-surface-500">
                        {installBusy ? "Running" : installSessionStatus === "success" ? "Completed" : "Failed"}
                      </span>
                    </div>
                    <div class="min-h-0 max-h-[55vh] flex-1 overflow-auto" bind:this={installLogContainer}>
                      {#if installLogText}
                        <pre class="min-w-0 whitespace-pre-wrap break-words px-3 py-3 font-mono text-[0.74rem] leading-5 text-surface-100">{installLogText}</pre>
                      {:else}
                        <p class="px-3 py-3 text-sm text-surface-400">Waiting for updater command output.</p>
                      {/if}
                    </div>
                  </section>
                </div>
              </section>
            {:else}
              <section class="min-w-0 rounded border border-surface-800/60 bg-surface-900/60 p-4">
                <div class="mb-3 flex flex-wrap items-center gap-2">
                  <p class="uppercase text-[0.58rem] tracking-[0.32em] text-surface-500">Network & USB</p>
                  <span class={`rounded border px-2 py-1 text-[0.56rem] font-semibold uppercase tracking-[0.25em] ${statusClass(selectedDevice.status)}`}>
                    {statusLabel(selectedDevice.status)}
                  </span>
                  {#each panelConnectionChips(selectedDevice) as chip (`detail-${selectedDevice.id}-${chip}`)}
                    <span class={`rounded border px-2 py-1 text-[0.56rem] uppercase tracking-[0.2em] ${chipClass(chip)}`}>
                      {chip}
                    </span>
                  {/each}
                </div>

                {#if networkFacts(selectedDevice).length}
                  <div class="grid min-w-0 gap-2">
                    {#each networkFacts(selectedDevice) as fact (`network-${selectedDevice.id}-${fact.label}`)}
                      <div class="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] items-center gap-3 rounded border border-surface-800/60 bg-surface-900/50 px-3 py-2">
                        <span class="shrink-0 text-[0.64rem] uppercase tracking-[0.22em] text-surface-500">{fact.label}</span>
                        <span class="min-w-0 truncate text-right font-mono text-sm text-surface-100">{fact.value}</span>
                      </div>
                    {/each}
                  </div>
                {:else}
                  <p class="text-sm text-surface-400">No network details available.</p>
                {/if}
              </section>

              <section class="min-w-0 rounded border border-surface-800/60 bg-surface-900/60 p-4">
                <p class="mb-3 uppercase text-[0.58rem] tracking-[0.32em] text-surface-500">Software</p>
                {#if softwareFacts(selectedDevice).length}
                  <div class="grid min-w-0 gap-2">
                    {#each softwareFacts(selectedDevice) as fact (`software-${selectedDevice.id}-${fact.label}`)}
                      <div class="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] items-center gap-3 rounded border border-surface-800/60 bg-surface-900/50 px-3 py-2">
                        <span class="shrink-0 text-[0.64rem] uppercase tracking-[0.22em] text-surface-500">{fact.label}</span>
                        <span class="min-w-0 truncate text-right font-mono text-sm text-surface-100">{fact.value}</span>
                      </div>
                    {/each}
                  </div>
                {:else}
                  <p class="text-sm text-surface-400">No software metadata yet.</p>
                {/if}

              </section>

              <section class="min-w-0 rounded border border-surface-800/60 bg-surface-900/60 p-4 lg:col-span-2">
                <div class="mb-3 flex items-center justify-between gap-3">
                  <p class="uppercase text-[0.58rem] tracking-[0.32em] text-surface-500">Live Telemetry</p>
                  <div class="text-right">
                    <span class="text-[0.62rem] uppercase tracking-[0.22em] text-surface-500">
                      {telemetryUpdatedLabel()}
                    </span>
                    {#if telemetryStreamMessage}
                      <p
                        class={`mt-1 text-[0.62rem] uppercase tracking-[0.18em] ${
                          telemetryStreamStatus === "error"
                            ? "text-error-200"
                            : telemetryStreamStatus === "live"
                              ? "text-success-200"
                              : "text-surface-500"
                        }`}
                      >
                        {telemetryStreamStatus === "live" ? "WebSocket live" : telemetryStreamMessage}
                      </p>
                    {/if}
                  </div>
                </div>

                {#if telemetryFacts(selectedDevice).length}
                  <div class="grid min-w-0 gap-2 sm:grid-cols-2 lg:grid-cols-3">
                    {#each telemetryFacts(selectedDevice) as fact (`telemetry-${selectedDevice.id}-${fact.label}`)}
                      <div class="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] items-center gap-3 rounded border border-surface-800/60 bg-surface-900/50 px-3 py-2">
                        <span class="shrink-0 text-[0.64rem] uppercase tracking-[0.22em] text-surface-500">{fact.label}</span>
                        <span class="min-w-0 truncate text-right font-mono text-sm text-surface-100">{fact.value}</span>
                      </div>
                    {/each}
                  </div>
                {:else}
                  <p class="text-sm text-surface-400">No telemetry reported yet.</p>
                {/if}
              </section>

              <section class="min-w-0 rounded border border-surface-800/60 bg-surface-900/60 p-4 lg:col-span-2">
                {#if supportsWpilibLogs(selectedDevice)}
                  <div class="mb-3 flex flex-wrap items-center justify-between gap-3">
                    <p class="uppercase text-[0.58rem] tracking-[0.32em] text-surface-500">WPILib Logs</p>
                    <div class="flex items-center gap-3">
                      <span class="text-[0.62rem] uppercase tracking-[0.22em] text-surface-500">
                        {wpilibLogsUpdatedLabel()}
                      </span>
                      <button
                        class="inline-flex h-8 w-8 items-center justify-center rounded border border-surface-700/80 bg-surface-900/80 text-surface-200 transition hover:border-primary-500/60 hover:text-primary-200 disabled:cursor-not-allowed disabled:opacity-50"
                        disabled={wpilibLogsInFlight || wpilibLogActionBusy}
                        type="button"
                        aria-label="Refresh WPILib logs"
                        title="Refresh WPILib logs"
                        onclick={() => void loadWpilibLogs()}
                      >
                        <svg class={`h-4 w-4 ${wpilibLogsInFlight ? "animate-spin" : ""}`} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                          <path d="M21 12a9 9 0 1 1-2.64-6.36"></path>
                          <polyline points="21 3 21 9 15 9"></polyline>
                        </svg>
                      </button>
                    </div>
                  </div>

                  <div class="mb-3 flex flex-wrap items-center gap-2">
                    <button
                      class="btn btn-3xs preset-tonal-surface uppercase tracking-[0.2em]"
                      type="button"
                      disabled={wpilibLogActionBusy || wpilibLogsInFlight || wpilibLogEntries.length === 0}
                      onclick={selectAllWpilibLogs}
                    >
                      Select All
                    </button>
                    <button
                      class="btn btn-3xs preset-tonal-surface uppercase tracking-[0.2em]"
                      type="button"
                      disabled={wpilibLogActionBusy || wpilibLogsInFlight || selectedWpilibLogIds.size === 0}
                      onclick={clearWpilibLogSelection}
                    >
                      Unselect All
                    </button>
                    <button
                      class="btn btn-3xs preset-tonal-secondary uppercase tracking-[0.2em]"
                      type="button"
                      disabled={wpilibLogActionBusy || wpilibLogsInFlight || selectedWpilibLogIds.size === 0}
                      onclick={() => void runWpilibLogAction("download")}
                    >
                      Download Selected
                    </button>
                    <button
                      class="btn btn-3xs preset-tonal-secondary uppercase tracking-[0.2em]"
                      type="button"
                      disabled={wpilibLogActionBusy || wpilibLogsInFlight || selectedWpilibLogIds.size === 0}
                      onclick={() => void runWpilibLogAction("delete")}
                    >
                      Delete Selected
                    </button>
                    <button
                      class="btn btn-3xs preset-tonal-secondary uppercase tracking-[0.2em]"
                      type="button"
                      disabled={wpilibLogActionBusy || wpilibLogsInFlight || selectedWpilibLogIds.size === 0}
                      onclick={() => void runWpilibLogAction("downloadDelete")}
                    >
                      Download + Delete
                    </button>
                    <span class="ml-auto text-xs text-surface-500">
                      Selected {selectedWpilibLogIds.size} / {wpilibLogEntries.length}
                    </span>
                  </div>

                  {#if wpilibLogsError}
                    <p class="mb-3 whitespace-pre-wrap rounded border border-error-500/40 bg-error-500/10 px-3 py-2 text-sm text-error-100">{wpilibLogsError}</p>
                  {/if}
                  {#if wpilibLogsMessage}
                    <p class="mb-2 text-xs text-surface-500">{wpilibLogsMessage}</p>
                  {/if}

                  <div class="h-72 overflow-auto rounded border border-surface-800/70 bg-surface-950/60">
                    {#if wpilibLogsBusy && wpilibLogEntries.length === 0}
                      <p class="px-3 py-3 text-sm text-surface-400">Loading WPILib logs...</p>
                    {:else if wpilibLogEntries.length > 0}
                      <table class="min-w-full text-left text-xs">
                        <thead class="sticky top-0 bg-surface-900/90 text-surface-400">
                          <tr>
                            <th class="px-3 py-2">Select</th>
                            <th class="px-3 py-2">File</th>
                            <th class="px-3 py-2 text-right">Size</th>
                            <th class="px-3 py-2">Modified</th>
                          </tr>
                        </thead>
                        <tbody>
                          {#each wpilibLogEntries as entry (entry.id)}
                            <tr class="border-t border-surface-800/60">
                              <td class="px-3 py-2">
                                <input
                                  type="checkbox"
                                  checked={selectedWpilibLogIds.has(entry.id)}
                                  disabled={wpilibLogActionBusy || wpilibLogsInFlight}
                                  onchange={() => toggleWpilibLogSelection(entry.id)}
                                />
                              </td>
                              <td class="px-3 py-2 font-mono text-surface-100">{entry.fileName}</td>
                              <td class="px-3 py-2 text-right font-mono text-surface-100">{formatBytes(entry.sizeBytes)}</td>
                              <td class="px-3 py-2 text-surface-300">
                                {entry.modifiedEpochMs ? new Date(entry.modifiedEpochMs).toLocaleString() : "Unknown"}
                              </td>
                            </tr>
                          {/each}
                        </tbody>
                      </table>
                    {:else}
                      <p class="px-3 py-3 text-sm text-surface-400">No WPILib logs found on roboRIO.</p>
                    {/if}
                  </div>
                {:else}
                  <div class="mb-3 flex items-center justify-between gap-3">
                    <p class="uppercase text-[0.58rem] tracking-[0.32em] text-surface-500">Device Log</p>
                    <div class="flex items-center gap-3">
                      <span class="text-[0.62rem] uppercase tracking-[0.22em] text-surface-500">
                        {deviceLogUpdatedLabel()}
                      </span>
                      <button
                        class="inline-flex h-8 w-8 items-center justify-center rounded border border-surface-700/80 bg-surface-900/80 text-surface-200 transition hover:border-primary-500/60 hover:text-primary-200 disabled:cursor-not-allowed disabled:opacity-50"
                        disabled={deviceLogInFlight || !supportsDeviceLog(selectedDevice)}
                        type="button"
                        aria-label="Refresh logs"
                        title="Refresh logs"
                        onclick={() => void loadDeviceLog()}
                      >
                        <svg class={`h-4 w-4 ${deviceLogInFlight ? "animate-spin" : ""}`} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                          <path d="M21 12a9 9 0 1 1-2.64-6.36"></path>
                          <polyline points="21 3 21 9 15 9"></polyline>
                        </svg>
                      </button>
                    </div>
                  </div>

                  {#if !supportsDeviceLog(selectedDevice)}
                    <p class="text-sm text-surface-400">Device logs are available for online HeliOS and PhotonVision devices.</p>
                  {:else if deviceLogError}
                    <p class="rounded border border-error-500/40 bg-error-500/10 px-3 py-2 text-sm text-error-100">{deviceLogError}</p>
                  {:else}
                    {#if deviceLogSourceUrl || deviceLogLineCount > 0}
                      <p class="mb-2 truncate text-xs text-surface-500">
                        {#if deviceLogSourceUrl}
                          Source: {deviceLogSourceUrl}
                        {/if}
                        {#if deviceLogLineCount > 0}
                          {deviceLogSourceUrl ? " · " : ""}{deviceLogLineCount} lines
                          {#if deviceLogTruncated}
                            (tail)
                          {/if}
                        {/if}
                      </p>
                    {/if}

                    <div class="h-72 overflow-auto rounded border border-surface-800/70 bg-surface-950/60">
                      {#if deviceLogBusy && !deviceLogText}
                        <p class="px-3 py-3 text-sm text-surface-400">Loading logs...</p>
                      {:else if deviceLogText}
                        <pre class="min-w-0 whitespace-pre-wrap break-words px-3 py-3 font-mono text-[0.72rem] leading-5 text-surface-100">{deviceLogText}</pre>
                      {:else}
                        <p class="px-3 py-3 text-sm text-surface-400">{deviceLogMessage ?? "No log lines returned."}</p>
                      {/if}
                    </div>
                  {/if}
                {/if}
              </section>
            {/if}
          </div>
        </div>
      </Panel>
    {:else}
      <Panel
        eyebrow="Selected Device"
        title="No device selected"
        subtitle="Run discovery and choose a device from the roster."
      />
    {/if}
  </div>
</div>

{#if showUpdateModal}
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-surface-950/80 px-4 py-6 backdrop-blur-sm"
    role="presentation"
    onclick={handleModalBackdropClick}
  >
    <div
      class="w-full max-w-2xl rounded border border-surface-700/70 bg-surface-900 shadow-[0_0_50px_-24px_rgba(0,0,0,1)]"
      role="dialog"
      aria-modal="true"
      aria-label={updateModalTitle}
    >
      <header class="flex items-start justify-between gap-4 border-b border-surface-800/70 px-5 py-4">
        <div>
          <p class="uppercase text-[0.58rem] tracking-[0.32em] text-surface-500">Image Installer</p>
          <h3 class="mt-1 text-lg font-semibold text-surface-50">{updateModalTitle}</h3>
          {#if selectedDevice}
            <p class="mt-1 text-xs text-surface-400">{selectedDevice.displayName} ({selectedDevice.id})</p>
          {/if}
        </div>
        <button
          class="inline-flex h-9 w-9 items-center justify-center rounded border border-surface-700/80 bg-surface-900/80 text-surface-200 transition hover:border-primary-500/60 hover:text-primary-200"
          type="button"
          aria-label="Close"
          onclick={closeUpdateModal}
        >
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M18 6L6 18M6 6l12 12"></path>
          </svg>
        </button>
      </header>

      <div class="space-y-5 px-5 py-5">
        {#if isInstallStateSelected}
          <fieldset class="space-y-2">
            <legend class="uppercase text-[0.56rem] tracking-[0.32em] text-surface-500">Operation</legend>
            <div class="grid gap-2 sm:grid-cols-2">
              <label class="flex cursor-pointer items-center gap-3 rounded border border-surface-700/70 bg-surface-900/80 px-3 py-3 text-sm text-surface-200">
                <input
                  type="radio"
                  name="install-action"
                  class="h-4 w-4"
                  checked={installAction === "mount"}
                  onchange={() => {
                    installAction = "mount";
                  }}
                  disabled={installBusy}
                />
                <span>Mount only</span>
              </label>
              <label class="flex cursor-pointer items-center gap-3 rounded border border-surface-700/70 bg-surface-900/80 px-3 py-3 text-sm text-surface-200">
                <input
                  type="radio"
                  name="install-action"
                  class="h-4 w-4"
                  checked={installAction === "flash"}
                  onchange={() => {
                    installAction = "flash";
                    if (releaseOptions.length === 0) {
                      void loadReleaseOptions();
                    }
                  }}
                  disabled={installBusy}
                />
                <span>Mount and flash image</span>
              </label>
            </div>
          </fieldset>
        {/if}

        {#if installAction === "flash" || installAction === "ota"}
          <fieldset class="space-y-2">
            <legend class="uppercase text-[0.56rem] tracking-[0.32em] text-surface-500">Image Source</legend>
            <div class="grid gap-2 sm:grid-cols-2">
              <label class="flex cursor-pointer items-center gap-3 rounded border border-surface-700/70 bg-surface-900/80 px-3 py-3 text-sm text-surface-200">
                <input
                  type="radio"
                  name="image-source"
                  class="h-4 w-4"
                  checked={imageSource === "release"}
                  onchange={() => {
                    imageSource = "release";
                  }}
                  disabled={installBusy}
                />
                <span>GitHub release image</span>
              </label>
              <label class="flex cursor-pointer items-center gap-3 rounded border border-surface-700/70 bg-surface-900/80 px-3 py-3 text-sm text-surface-200">
                <input
                  type="radio"
                  name="image-source"
                  class="h-4 w-4"
                  checked={imageSource === "local"}
                  onchange={() => {
                    imageSource = "local";
                  }}
                  disabled={installBusy}
                />
                <span>Local image file</span>
              </label>
            </div>
          </fieldset>

          {#if imageSource === "release"}
            <div class="space-y-2">
              <div class="flex items-center justify-between gap-3">
                <label class="uppercase text-[0.56rem] tracking-[0.32em] text-surface-500" for="release-select">
                  GitHub Release
                </label>
                <button
                  class="inline-flex h-8 w-8 items-center justify-center rounded border border-surface-700/80 bg-surface-900/80 text-surface-200 transition hover:border-primary-500/60 hover:text-primary-200 disabled:cursor-not-allowed disabled:opacity-50"
                  type="button"
                  aria-label="Refresh releases"
                  title="Refresh releases"
                  disabled={releasesLoading || installBusy}
                  onclick={() => void loadReleaseOptions()}
                >
                  <svg class={`h-4 w-4 ${releasesLoading ? "animate-spin" : ""}`} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path d="M21 12a9 9 0 1 1-2.64-6.36"></path>
                    <polyline points="21 3 21 9 15 9"></polyline>
                  </svg>
                </button>
              </div>

              <select
                id="release-select"
                class="w-full rounded border border-surface-700/70 bg-surface-900 px-3 py-2 text-sm text-surface-100 disabled:cursor-not-allowed disabled:opacity-60"
                bind:value={selectedReleaseUrl}
                disabled={releasesLoading || installBusy || releaseOptions.length === 0}
              >
                {#if releaseOptions.length === 0}
                  <option value="">{releasesLoading ? "Loading releases..." : "No release images found"}</option>
                {:else}
                  {#each releaseOptions as option (option.downloadUrl)}
                    <option value={option.downloadUrl}>
                      {option.releaseTag} · {option.assetName} ({formatBytes(option.sizeBytes)})
                    </option>
                  {/each}
                {/if}
              </select>

              {#if selectedReleaseUrl}
                {@const selectedRelease = releaseOptions.find((option) => option.downloadUrl === selectedReleaseUrl)}
                {#if selectedRelease}
                  <p class="text-xs text-surface-400">
                    {selectedRelease.releaseName}
                    {#if selectedRelease.prerelease}
                      · prerelease
                    {/if}
                    {#if selectedRelease.publishedAt}
                      · published {new Date(selectedRelease.publishedAt).toLocaleString()}
                    {/if}
                  </p>
                {/if}
              {/if}

              {#if releasesError}
                <p class="rounded border border-error-500/40 bg-error-500/10 px-3 py-2 text-xs text-error-100">
                  {releasesError}
                </p>
              {/if}
            </div>
          {:else}
            <div class="space-y-2">
              <p class="uppercase text-[0.56rem] tracking-[0.32em] text-surface-500">Local Image File</p>
              <div class="flex items-center gap-2">
                <div class="min-w-0 flex-1 rounded border border-surface-700/70 bg-surface-900 px-3 py-2 text-sm text-surface-100">
                  <p class="truncate">
                    {localImageFileName || "No file selected"}
                  </p>
                </div>
                <button
                  class="btn btn-3xs preset-tonal-surface uppercase tracking-[0.2em]"
                  type="button"
                  disabled={installBusy}
                  onclick={() => void pickLocalImageFile()}
                >
                  Choose File
                </button>
              </div>
              {#if localImagePath}
                <p class="truncate text-xs text-surface-500">{localImagePath}</p>
              {/if}
            </div>
          {/if}
        {/if}

        <div class="rounded border border-surface-800/70 bg-surface-950/60 px-3 py-3 text-xs text-surface-300">
          {#if installAction === "mount"}
            Mount-only runs: `rpiboot` -> rediscover flash target. No image write.
          {:else if installAction === "ota"}
            OTA runs: resolve image -> upload image -> schedule OTA apply -> monitor OTA progress -> wait for reboot and reconnect.
          {:else}
            Apply image runs: `rpiboot` -> rediscover flash target -> image flash.
          {/if}
        </div>

        {#if installError}
          <div class="rounded border border-error-500/40 bg-error-500/10 px-3 py-3 text-sm text-error-100">
            {installError}
          </div>
        {/if}

        {#if installResult}
          <div class="space-y-2">
            <div
              class={`rounded border px-3 py-3 text-sm ${
                installResult.success
                  ? "border-success-500/40 bg-success-500/10 text-success-100"
                  : "border-error-500/40 bg-error-500/10 text-error-100"
              }`}
            >
              <p>{installResult.message}</p>
              <p class="mt-2 text-xs opacity-90">
                Mode: {installResult.mode}
                {#if installResult.imagePath}
                  · Image: {installResult.imagePath}
                {/if}
                {#if installResult.selectedTargetPath}
                  · Target: {installResult.selectedTargetPath}
                {/if}
              </p>
            </div>
          </div>
        {/if}

        <p class="text-xs text-surface-500">
          Apply closes this modal and starts live updater status/log streaming in the selected device info panel.
        </p>
      </div>

      <footer class="flex items-center justify-end gap-3 border-t border-surface-800/70 px-5 py-4">
        <button
          class="btn btn-3xs preset-tonal-surface uppercase tracking-[0.2em]"
          type="button"
          disabled={installBusy}
          onclick={closeUpdateModal}
        >
          Close
        </button>
        {#if installAction === "ota"}
          <div class="relative">
            <button
              class="btn btn-3xs preset-tonal-secondary inline-flex items-center gap-2 uppercase tracking-[0.2em] disabled:cursor-not-allowed disabled:opacity-60"
              type="button"
              disabled={!canApplyInstall}
              aria-haspopup="menu"
              aria-expanded={otaApplyMenuOpen}
              onclick={toggleOtaApplyMenu}
            >
              {#if installBusy}
                Updating...
              {:else}
                Apply OTA Image
              {/if}
              <span aria-hidden="true">{otaApplyMenuOpen ? "▲" : "▼"}</span>
            </button>

            {#if otaApplyMenuOpen}
              <div
                class="absolute right-0 top-[calc(100%+0.45rem)] z-50 w-full min-w-full rounded border border-surface-700/80 bg-surface-950/98 p-1.5 shadow-[0_18px_45px_-22px_rgba(0,0,0,0.9)]"
                role="menu"
                aria-label="OTA apply transport"
              >
                <button
                  class="flex w-full items-center rounded px-3 py-2 text-left text-xs font-semibold uppercase tracking-[0.2em] text-surface-100 transition hover:bg-surface-800/70 disabled:cursor-not-allowed disabled:opacity-45"
                  type="button"
                  role="menuitem"
                  disabled={!canApplyOtaViaApi}
                  onclick={() => chooseOtaApplyTransport("api")}
                >
                  Updater over API
                </button>
                <button
                  class="mt-1 flex w-full items-center rounded px-3 py-2 text-left text-xs font-semibold uppercase tracking-[0.2em] text-surface-100 transition hover:bg-surface-800/70 disabled:cursor-not-allowed disabled:opacity-45"
                  type="button"
                  role="menuitem"
                  disabled={!canApplyOtaViaUsb}
                  onclick={() => chooseOtaApplyTransport("usb")}
                >
                  Updater over USB
                </button>
              </div>
            {/if}
          </div>
        {:else}
          <button
            class="btn btn-3xs preset-tonal-secondary uppercase tracking-[0.2em] disabled:cursor-not-allowed disabled:opacity-60"
            type="button"
            disabled={!canApplyInstall}
            onclick={() => void applySelectedInstallAction()}
          >
            {#if installBusy}
              {installAction === "mount" ? "Mounting..." : "Applying..."}
            {:else}
              {installAction === "mount" ? "Mount Device" : "Apply Image"}
            {/if}
          </button>
        {/if}
      </footer>
    </div>
  </div>
{/if}
