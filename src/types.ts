// 与 Rust 后端对应的数据类型

export type ProcKind = "critical" | "service" | "system" | "user";

export interface ProcessInfo {
  pid: number;
  ppid: number;
  name: string;
  createTime: string;
  startTime: number;
  cpu: number;
  memory: number;
  workingSet: number;
  privateBytes: number;
  gpu: number | null;
  gpuMem: number | null;
  ioRead: number;
  ioWrite: number;
  threads: number;
  handles: number;
  sessionId: number;
  path: string | null;
  kind: ProcKind;
  status: "normal" | "abnormal" | "protected" | "exited";
  score: number;
  whitelisted: boolean;
  ignored: boolean;
}

export interface CpuState {
  usage: number;
  perCore: number[];
  temp: number | null;
  power: number | null;
  clock: number | null;
  clockSource: string;
}

export interface GpuState {
  name: string;
  vendor: string;
  luid: string;
  integrated: boolean;
  usage: number | null;
  engines: [string, number][];
  temp: number | null;
  hotspot: number | null;
  power: number | null;
  clock: number | null;
  memClock: number | null;
  memUsed: number | null;
  memTotal: number | null;
  memUsage: number | null;
  sharedUsed: number | null;
  fanRpm: number | null;
  fanPercent: number | null;
}

export interface MemoryStatus {
  total: number;
  used: number;
  available: number;
  usage: number;
  commitTotal: number;
  commitLimit: number;
  pagefileUsage: number | null;
  cached: number;
}

export interface FanReading {
  name: string;
  hardware: string;
  kind: "cpu" | "gpu" | "case";
  rpm: number;
}

export interface TempReading {
  name: string;
  hardware: string;
  hwType: string;
  value: number;
  max: number | null;
}

export interface SysPoint {
  ts: number;
  cpu: number;
  gpu: number | null;
  mem: number;
  cpuTemp: number | null;
  gpuTemp: number | null;
  cpuPower: number | null;
  gpuPower: number | null;
  fan: number | null;
  diskTemp: number | null;
  diskActive: number | null;
}

export interface Marker {
  ts: number;
  title: string;
  level: string;
}

export interface AnomalyView {
  kind: string;
  code: string;
  title: string;
  level: "notice" | "high" | "critical";
  since: number;
  durationSecs: number;
  value: number | null;
  threshold: number | null;
  description: string;
  ignored: boolean;
  relatedPid: number | null;
  relatedName: string | null;
}

export interface HwinfoMgrStatus {
  auto: boolean;
  stage: "disabled" | "notInstalled" | "importing" | "ready" | "launching" | "running" | "declined" | "error";
  path: string | null;
  managed: boolean;
  running: boolean;
  launchedByUs: boolean;
  configured: boolean | null;
  message: string | null;
  updatedAt: number;
}

export interface SensorStatus {
  status: "notFound" | "starting" | "running" | "exited" | "paused" | "disabled";
  admin: boolean;
  message: string | null;
  lhmVersion: string | null;
  updatedAt: number;
  nvml: boolean;
  source: string;
  mode: "auto" | "lhm" | "hwinfo";
  hwinfo: "disabled" | "notFound" | "inactive" | "invalid" | "stale" | "running";
  hwinfoSensors: number;
  ec: "available" | "needAdmin" | "none";
}

export interface Snapshot {
  ts: number;
  cpu: CpuState;
  gpus: GpuState[];
  gpuUsage: number | null;
  gpuTemp: number | null;
  gpuPower: number | null;
  gpuAvailable: boolean;
  memory: MemoryStatus;
  disk: {
    read: number | null;
    write: number | null;
    active: number | null;
    temp: number | null;
    tempName: string | null;
  };
  fans: FanReading[];
  fanRpm: number | null;
  temps: TempReading[];
  processes: ProcessInfo[];
  processCount: number;
  threadCount: number;
  handleCount: number;
  anomalies: AnomalyView[];
  level: "normal" | "notice" | "high" | "critical";
  levelText: string;
  sensor: SensorStatus;
  hwinfoMgr: HwinfoMgrStatus;
  newMarkers: Marker[];
  point: SysPoint;
}

export interface GpuAdapter {
  name: string;
  vendor: string;
  luid: string;
  dedicatedTotal: number;
  sharedTotal: number;
  integrated: boolean;
}

export interface StaticInfo {
  cpuModel: string;
  cpuVendor: string;
  cores: number;
  threads: number;
  baseMhz: number | null;
  totalMemory: number;
  gpus: GpuAdapter[];
  osName: string;
  osBuild: string;
  hostname: string;
}

export interface Settings {
  autostart: boolean;
  minimizeToTray: boolean;
  notifications: boolean;
  autoStartMonitoring: boolean;
  runAsAdmin: boolean;
  theme: "dark" | "light" | "system";
  processIntervalMs: number;
  sensorSource: "auto" | "lhm" | "hwinfo";
  hwinfoAuto: boolean;
  sensorIntervalMs: number;
  historyIntervalMs: number;
  cpuThreshold: number;
  gpuThreshold: number;
  cpuTempThreshold: number;
  gpuTempThreshold: number;
  diskTempThreshold: number;
  fanRpmThreshold: number;
  sustainSeconds: number;
  historyRetentionDays: number;
  whitelist: string[];
  ignoreList: string[];
  showSystemProcesses: boolean;
  showServices: boolean;
  confirmKill: boolean;
}

export interface ScoreBreakdown {
  cpu: number;
  gpu: number;
  memory: number;
  power: number;
  sustain: number;
  weights: [number, number, number, number, number];
  powerSource: "power" | "temperature" | "unavailable";
  gpuAvailable: boolean;
  avgCpu: number;
  avgGpu: number;
  memGrowth: number;
  sustainedSecs: number;
  windowSecs: number;
}

export interface Suspect {
  pid: number;
  createTime: string;
  name: string;
  path: string | null;
  cpuAvg: number;
  gpuAvg: number | null;
  memory: number;
  durationSecs: number;
  score: number;
  grade: "normal" | "attention" | "high" | "critical";
  gradeText: string;
  correlation: string;
  reasons: string[];
  breakdown: ScoreBreakdown;
  protected: boolean;
  whitelisted: boolean;
  kind: ProcKind;
}

export interface Suggestion {
  title: string;
  detail: string;
  relatedPid: number | null;
  relatedName: string | null;
}

export interface DiagnosisReport {
  generatedAt: number;
  level: string;
  levelText: string;
  title: string;
  durationSecs: number | null;
  anomalies: AnomalyView[];
  suspects: Suspect[];
  hardware: SysPoint;
  reasons: string[];
  suggestions: Suggestion[];
  dataNotes: string[];
  windowSecs: number;
}

export interface ProcBrief {
  pid: number;
  createTime: string;
  name: string;
  cpu: number;
  memory: number;
  gpu: number | null;
}

export interface FileInfo {
  size: number | null;
  modified: number | null;
  created: number | null;
  description: string | null;
  company: string | null;
  product: string | null;
  version: string | null;
  copyright: string | null;
  originalName: string | null;
}

export interface SignatureInfo {
  status: "valid" | "notSigned" | "hashMismatch" | "notTrusted" | "unknownError" | "unavailable";
  statusText: string;
  signer: string | null;
  issuer: string | null;
  signatureType: string | null;
  notAfter: string | null;
  thumbprint: string | null;
}

export interface ProcessDetail {
  info: ProcessInfo;
  alive: boolean;
  parent: ProcBrief | null;
  children: ProcBrief[];
  history: [number, number, number, number][];
  gpuEngines: { adapter: string; engine: string; usage: number }[];
  gpuSupported: boolean;
  score: number;
  breakdown: ScoreBreakdown | null;
  suspect: Suspect | null;
  file: FileInfo | null;
  kindText: string;
  elevated: boolean;
  logicalProcessors: number;
}

export interface ThreadInfo {
  tid: number;
  cpuTimeMs: number;
  kernelTimeMs: number;
  userTimeMs: number;
  startTime: number;
  priority: number;
  basePriority: number;
  contextSwitches: number;
  state: string;
  waitReason: string;
  startAddress: string;
}

export interface KillPreview {
  pid: number;
  name: string;
  allowed: boolean;
  blockedReason: string | null;
  warning: "none" | "service" | "sensitive";
  warningText: string | null;
  hasWindows: boolean;
  children: { pid: number; name: string; protected: boolean }[];
}

export interface KillResult {
  success: boolean;
  message: string;
  requiresAdmin: boolean;
  canForce: boolean;
  killed: number[];
  failed: { pid: number; name: string; reason: string }[];
  processName: string;
}

export interface HardwareRow {
  timestamp: number;
  cpuUsage: number | null;
  cpuTemp: number | null;
  cpuPower: number | null;
  gpuUsage: number | null;
  gpuTemp: number | null;
  gpuPower: number | null;
  memoryUsage: number | null;
  fanRpm: number | null;
  diskTemp: number | null;
}

export interface EventRow {
  id: number;
  timestamp: number;
  endedAt: number | null;
  level: string;
  eventType: string;
  description: string;
  relatedPid: number | null;
  relatedName: string | null;
  status: string;
}

export interface ActionRow {
  id: number;
  timestamp: number;
  pid: number;
  processName: string;
  action: string;
  result: string;
}

export interface ProcessAggRow {
  processName: string;
  avgCpu: number;
  maxCpu: number;
  avgMemory: number;
  avgGpu: number;
  samples: number;
  lastSeen: number;
}

export interface HistoryData {
  from: number;
  to: number;
  hardware: HardwareRow[];
  events: EventRow[];
  actions: ActionRow[];
  processes: ProcessAggRow[];
}

export interface SensorNode {
  id: string;
  name: string;
  type: string;
  value: number | null;
  min: number | null;
  max: number | null;
  unit?: string | null;
}

export interface HwNode {
  id: string;
  name: string;
  type: string;
  parent: string | null;
  parentType?: string | null;
  parentName?: string | null;
  sensors: SensorNode[];
}

export type KillMode = "graceful" | "force" | "tree";
