export type QueryType = 'phone' | 'bank' | 'url';

export interface AgentSummary {
  source: string;
  summary: string;
  key_facts: string[];
  phone_mentions: string[];
  risk_signals: string[];
  evidence_urls: string[];
}

export interface AgentExtraction {
  url: string;
  summary: string;
  entities: string[];
  risk_signals: string[];
  related_numbers: string[];
}

export interface SelectedUrl {
  url: string;
  reason: string;
  priority: number;
}

export type RiskLevel = 'critical' | 'high' | 'medium' | 'low' | 'unknown';

export interface HistoricalContextEvent {
  investigation_count: number;
  last_risk_level: RiskLevel | string;
  approved_reports: number;
  linked_subjects_count: number;
  first_seen: string;
  last_seen: string;
}

export interface NetworkNode {
  id: string;
  value: string;
  subject_type: QueryType;
  risk_level: RiskLevel | string;
  risk_score: number;
  depth: number;
}

export interface NetworkEdge {
  subject_a_id: string;
  subject_b_id: string;
  link_type: string;
  strength: number;
}

export interface NetworkGraph {
  root_subject_id: string;
  nodes: NetworkNode[];
  edges: NetworkEdge[];
}

// SSE event payloads
export interface InvestigationStartedEvent { investigation_id: string; }
export interface PhaseStartEvent { phase: number; label: string; total_sources?: number; }
export interface SourceStatusEvent { source: string; status: string; found: number; message?: string; }
export interface ProgressEvent { phase: number; message: string; }
export interface SummaryResultEvent { source: string; result: AgentSummary; }
export interface UrlAssessmentEvent { selected: number; total: number; urls: SelectedUrl[]; }
export interface ExtractionResultEvent { url: string; result: AgentExtraction; }
export interface HistoricalContextSseEvent extends HistoricalContextEvent {}
export interface DetectiveStreamEvent { chunk: string; done: boolean; replace: boolean; }
export interface CompleteEvent { investigation_id: string; risk_level: RiskLevel; confidence: number; sources_analyzed: number; duration_ms: number; }
export interface ErrorEvent { phase?: number; message: string; recoverable: boolean; }

export type NarrativeLineType = 'text' | 'evidence-summary' | 'evidence-extraction';

export interface NarrativeLine {
  id: number;
  type: NarrativeLineType;
  text?: string;
  icon?: string;
  timestamp?: string;
  summary?: SummaryResultEvent;
  extraction?: ExtractionResultEvent;
  animate?: boolean;
}
