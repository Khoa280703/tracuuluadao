import type {
  CompleteEvent,
  DetectiveStreamEvent,
  ErrorEvent,
  ExtractionResultEvent,
  HistoricalContextSseEvent,
  InvestigationStartedEvent,
  PhaseStartEvent,
  ProgressEvent,
  QueryType,
  SourceStatusEvent,
  SummaryResultEvent,
  UrlAssessmentEvent,
} from './types';
import { buildApiUrl } from './api-base';

export interface SSECallbacks {
  onInvestigationStarted: (event: InvestigationStartedEvent) => void;
  onPhaseStart: (event: PhaseStartEvent) => void;
  onSourceStatus: (event: SourceStatusEvent) => void;
  onProgress: (event: ProgressEvent) => void;
  onSummaryResult: (event: SummaryResultEvent) => void;
  onUrlAssessment: (event: UrlAssessmentEvent) => void;
  onExtractionResult: (event: ExtractionResultEvent) => void;
  onHistoricalContext: (event: HistoricalContextSseEvent) => void;
  onDetectiveStream: (event: DetectiveStreamEvent) => void;
  onComplete: (event: CompleteEvent) => void;
  onError: (event: ErrorEvent) => void;
}

export interface ReportStreamCallbacks {
  onDetectiveStream: (event: DetectiveStreamEvent) => void;
  onError: (event: ErrorEvent) => void;
}

export function createInvestigation(query: string, type: QueryType, callbacks: SSECallbacks) {
  const url = buildApiUrl(
    `/api/investigate?q=${encodeURIComponent(query)}&type=${type}`,
  );
  const es = new EventSource(url);
  let completed = false;

  es.addEventListener('investigation_started', (e: MessageEvent) => {
    try { callbacks.onInvestigationStarted(JSON.parse(e.data)); } catch { /* skip */ }
  });
  es.addEventListener('phase_start', (e: MessageEvent) => {
    try { callbacks.onPhaseStart(JSON.parse(e.data)); } catch { /* skip */ }
  });
  es.addEventListener('source_status', (e: MessageEvent) => {
    try { callbacks.onSourceStatus(JSON.parse(e.data)); } catch { /* skip */ }
  });
  es.addEventListener('progress', (e: MessageEvent) => {
    try { callbacks.onProgress(JSON.parse(e.data)); } catch { /* skip */ }
  });
  es.addEventListener('summary_result', (e: MessageEvent) => {
    try { callbacks.onSummaryResult(JSON.parse(e.data)); } catch { /* skip */ }
  });
  es.addEventListener('url_assessment', (e: MessageEvent) => {
    try { callbacks.onUrlAssessment(JSON.parse(e.data)); } catch { /* skip */ }
  });
  es.addEventListener('extraction_result', (e: MessageEvent) => {
    try { callbacks.onExtractionResult(JSON.parse(e.data)); } catch { /* skip */ }
  });
  es.addEventListener('historical_context', (e: MessageEvent) => {
    try { callbacks.onHistoricalContext(JSON.parse(e.data)); } catch { /* skip */ }
  });
  es.addEventListener('detective_stream', (e: MessageEvent) => {
    try { callbacks.onDetectiveStream(JSON.parse(e.data)); } catch { /* skip */ }
  });
  es.addEventListener('complete', (e: MessageEvent) => {
    try {
      completed = true;
      callbacks.onComplete(JSON.parse(e.data));
      es.close();
    } catch { /* skip */ }
  });
  es.addEventListener('investigation_error', (e: MessageEvent) => {
    try { callbacks.onError(JSON.parse(e.data)); } catch { /* skip */ }
  });

  es.onerror = () => {
    if (!completed) {
      es.close();
      callbacks.onError({ message: 'Kết nối cập nhật kết quả đã bị gián đoạn.', recoverable: false });
    }
  };

  return { close: () => es.close() };
}

export function createInvestigationReportStream(
  investigationId: string,
  callbacks: ReportStreamCallbacks,
) {
  const url = buildApiUrl(
    `/api/investigate/report?investigation_id=${encodeURIComponent(investigationId)}`,
  );
  const es = new EventSource(url);
  let finished = false;

  es.addEventListener('detective_stream', (e: MessageEvent) => {
    try {
      const event = JSON.parse(e.data) as DetectiveStreamEvent;
      callbacks.onDetectiveStream(event);
      if (event.done) {
        finished = true;
        es.close();
      }
    } catch {
      /* skip */
    }
  });
  es.addEventListener('investigation_error', (e: MessageEvent) => {
    try {
      finished = true;
      callbacks.onError(JSON.parse(e.data));
      es.close();
    } catch {
      /* skip */
    }
  });

  es.onerror = () => {
    if (!finished) {
      es.close();
      callbacks.onError({ phase: 5, message: 'Kết nối tải báo cáo đã bị gián đoạn.', recoverable: false });
    }
  };

  return { close: () => es.close() };
}
