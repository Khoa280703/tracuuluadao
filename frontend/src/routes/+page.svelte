<script lang="ts">
  import { tick } from 'svelte';
  import { buildApiUrl } from '$lib/api-base';
  import { sanitizeMarkdown, sanitizeReportMarkdown } from '$lib/markdown';
  import { createInvestigation, createInvestigationReportStream } from '$lib/sse-client';
  import type {
    CompleteEvent,
    ExtractionResultEvent,
    HistoricalContextSseEvent,
    InvestigationStartedEvent,
    NetworkGraph,
    QueryType,
    RiskLevel,
    SourceStatusEvent,
    SummaryResultEvent,
    UrlAssessmentEvent,
  } from '$lib/types';

  let { data } = $props<{ data: { q: string; type: QueryType } }>();

  type PageState = 'home' | 'investigating';
  type InvestigationHandle = ReturnType<typeof createInvestigation>;
  type ReportStreamHandle = ReturnType<typeof createInvestigationReportStream>;
  type TabKey = 'phone' | 'bank' | 'url' | 'email' | 'other';
  type InvestigationPanelTab = 'process' | 'report';
  type TimelineStatus = 'pending' | 'active' | 'complete';
  type PhaseLogTone = 'info' | 'success' | 'warning';
  type PhaseLogEntry = {
    id: number;
    text: string;
    timestamp: string;
    tone: PhaseLogTone;
    replaceKey?: string;
  };
  type TimelinePhase = {
    phase: number;
    label: string;
    startedAt: string | null;
    totalSources?: number;
  };

  const baseTimelinePhases: TimelinePhase[] = [
    { phase: 1, label: 'Thu thập dữ liệu', startedAt: null },
    { phase: 2, label: 'Tóm tắt nguồn', startedAt: null },
    { phase: 3, label: 'Chọn nguồn cần xem kỹ', startedAt: null },
    { phase: 4, label: 'Phân tích nội dung chi tiết', startedAt: null },
    { phase: 5, label: 'Tổng hợp điều tra', startedAt: null },
  ];

  const riskPresentation: Record<RiskLevel, { title: string; description: string; panel: string; iconWrap: string; icon: string; bar: string }> = {
    critical: {
      title: 'Nguy cơ rất cao',
      description: 'Phát hiện nhiều tín hiệu lừa đảo nghiêm trọng, cần tránh giao dịch ngay.',
      panel: 'bg-red-50/70 border-red-200',
      iconWrap: 'bg-red-100 text-red-600',
      icon: 'fa-solid fa-triangle-exclamation',
      bar: 'bg-red-500',
    },
    high: {
      title: 'Nguy cơ cao',
      description: 'Có nhiều dấu hiệu rủi ro và cần kiểm tra kỹ trước khi tương tác.',
      panel: 'bg-orange-50/70 border-orange-200',
      iconWrap: 'bg-orange-100 text-orange-600',
      icon: 'fa-solid fa-shield-exclamation',
      bar: 'bg-orange-500',
    },
    medium: {
      title: 'Nguy cơ trung bình',
      description: 'Đã phát hiện một số tín hiệu đáng ngờ, nên thẩm định thêm.',
      panel: 'bg-amber-50/70 border-amber-200',
      iconWrap: 'bg-amber-100 text-amber-600',
      icon: 'fa-solid fa-circle-exclamation',
      bar: 'bg-amber-500',
    },
    low: {
      title: 'Nguy cơ thấp',
      description: 'Chưa thấy nhiều tín hiệu bất thường, nhưng vẫn nên thận trọng.',
      panel: 'bg-emerald-50/70 border-emerald-200',
      iconWrap: 'bg-emerald-100 text-emerald-600',
      icon: 'fa-solid fa-circle-check',
      bar: 'bg-emerald-500',
    },
    unknown: {
      title: 'Chưa đủ dữ liệu',
      description: 'Nguồn dữ liệu còn hạn chế, kết luận hiện tại chỉ mang tính tham khảo.',
      panel: 'bg-slate-50/70 border-slate-200',
      iconWrap: 'bg-slate-100 text-slate-600',
      icon: 'fa-solid fa-circle-question',
      bar: 'bg-slate-500',
    },
  };

  let pageState = $state<PageState>('home');
  let currentQuery = $state('');
  let currentType = $state<QueryType>('phone');
  let searchQuery = $state('');
  let selectedTab = $state<TabKey>('phone');
  let investigationPanelTab = $state<InvestigationPanelTab>('process');
  let investigationHandle = $state<InvestigationHandle | null>(null);
  let reportHandle = $state<ReportStreamHandle | null>(null);
  let currentInvestigationId = $state('');
  let processScrollViewport = $state<HTMLElement | null>(null);
  let reportScrollViewport = $state<HTMLElement | null>(null);
  let processNarrativeTarget = $state('');
  let processNarrativeVisible = $state('');
  let timelinePhases = $state(createInitialTimeline());
  let sourceStatuses = $state<SourceStatusEvent[]>([]);
  let summaryResults = $state<SummaryResultEvent[]>([]);
  let urlAssessment = $state<UrlAssessmentEvent | null>(null);
  let extractionResults = $state<ExtractionResultEvent[]>([]);
  let detectiveNarrative = $state('');
  let detectiveNarrativeVisible = $state('');
  let reportNarrativePrimed = $state(false);
  let phaseLogs = $state<Record<number, PhaseLogEntry[]>>({});
  let progressMessages = $state<Record<number, string>>({});
  let completeEvent = $state<CompleteEvent | null>(null);
  let investigationError = $state<string | null>(null);
  let reportStreamError = $state<string | null>(null);
  let historicalContext = $state<HistoricalContextSseEvent | null>(null);
  let linkedSubjectsGraph = $state<NetworkGraph | null>(null);
  let linkedSubjectsLoading = $state(false);
  let linkedSubjectsError = $state<string | null>(null);
  let reportDescription = $state('');
  let reportCategory = $state('scam');
  let reportSubmitting = $state(false);
  let reportSubmitMessage = $state<string | null>(null);
  let reportSubmitError = $state<string | null>(null);
  let autoStarted = false;
  let phaseLogSequence = 0;
  let processNarrativeTimer: ReturnType<typeof setTimeout> | null = null;
  let reportNarrativeTimer: ReturnType<typeof setTimeout> | null = null;
  let reportStreamStartTimer: ReturnType<typeof setTimeout> | null = null;
  let streamedProcessPhaseHeadings = new Set<number>();
  let streamedProcessEntries = new Set<string>();
  let processNarrativeReplaceMap = new Map<string, string>();
  let linkedSubjectsAbortController: AbortController | null = null;
  let linkedSubjectsRequestId = 0;

  const reportCategoryOptions = [
    { value: 'scam', label: 'Nghi lừa đảo' },
    { value: 'spam', label: 'Spam / quấy rối' },
    { value: 'impersonation', label: 'Mạo danh' },
    { value: 'payment', label: 'Dụ chuyển tiền' },
  ];

  const tabItems = [
    { key: 'phone' as const, label: 'Số điện thoại', icon: 'fa-phone' },
    { key: 'bank' as const, label: 'Số tài khoản', icon: 'fa-building-columns' },
    { key: 'url' as const, label: 'Website', icon: 'fa-link' },
    { key: 'email' as const, label: 'Email', icon: 'fa-envelope' },
    { key: 'other' as const, label: 'Mã số khác', icon: 'fa-barcode' },
  ];

  const tabPlaceholders: Record<string, string> = {
    phone: 'Nhập số điện thoại cần tra cứu (VD: 09xxxxxxx)',
    bank: 'Nhập số tài khoản ngân hàng cần tra cứu',
    url: 'Nhập website hoặc tên miền cần tra cứu',
    email: 'Nhập địa chỉ email cần tra cứu',
    other: 'Nhập mã số cần tra cứu',
  };

  const featureCards = [
    { icon: 'fa-shield-check', title: 'Bảo mật tuyệt đối', desc: 'Cam kết không lưu trữ thông tin cá nhân của bạn.', color: 'blue', badge: 'An toàn 100%' },
    { icon: 'fa-bolt', title: 'Truy quét mạnh mẽ', desc: 'Sử dụng AI và hàng trăm công cụ để tìm kiếm thông tin nhanh chóng.', color: 'green', badge: 'Nhanh chóng & Chính xác' },
    { icon: 'fa-bullseye', title: 'Báo cáo dễ hiểu', desc: 'Kết quả được trình bày rõ ràng, dễ hiểu cho mọi đối tượng.', color: 'purple', badge: 'Dễ hiểu & Trực quan' },
    { icon: 'fa-lock', title: 'Miễn phí cơ bản', desc: 'Tra cứu cơ bản miễn phí, nâng cao với nhiều ưu đãi.', color: 'orange', badge: 'Linh hoạt & Tiết kiệm' },
  ];

  const stats = [
    { icon: 'fa-users', value: '150K+', label: 'Người dùng tin tưởng', color: 'blue' },
    { icon: 'fa-magnifying-glass', value: '2.5M+', label: 'Lượt tra cứu thành công', color: 'blue' },
    { icon: 'fa-shield-check', value: '98.6%', label: 'Độ chính xác thông tin', color: 'blue' },
    { icon: 'fa-clock', value: '30s', label: 'Thời gian trả kết quả trung bình', color: 'purple', regular: true },
  ];

  const avatarUrls = [
    'https://lh3.googleusercontent.com/aida-public/AB6AXuDFFBsqIH736MqDM-zYncF0IUN5Eoc0FxvjsSx5C6VrO3FAXxlJDuIe2rpQv_6qs7s1G-m58Z0_6flR2mJjcMeS-CQoLgTCH0BaWXmQiF2BFZPa29EOqjIXzWxPt85TAbCNY2DIOte2dgcNBQ0gRiRfWhXO3wiezHm_U6dKTbgABmZR1_xBE_Q8Rdj8BfLRWiGATkSq-jeWEPJl4JKOiwCI93pa-vOBby86lwNa0ngrL_sr-8p_SG9HPk2f6jGfmkWHLy6V2hzEIBA',
    'https://lh3.googleusercontent.com/aida-public/AB6AXuBWt7eKBy56V_6BKivPBKx6av8s3bXq3Irb6qiAxYeFNhG03rInj5yIEYwm7hvtjGRiREg1gY4ZX3PZbtG-OfF04QT3a99XziJisVrLOwM0-hNVazSfCUXlNY0XsSc6QxfCbsBoGeQWMG6VqA3ZtJkOYZzZwnMbn1X6nPXEz5-5QgORXhjZaZOfzNb9NS3xzga3QhGym-A5naXOsmI2Xulke-XVl4RM3LicCXEsPzMUvAvviFM6wilnmPdbq6_RYya-on9u5ruQrKU',
    'https://lh3.googleusercontent.com/aida-public/AB6AXuBOaRI-guWZoAu-nKH0uoMNGd1ELrkjaQx7VEiUblW2R7OQESjkj21uxt9hpyjwhsBewF7xzYAaBQYdB9g2Jd1JjLRPr8nDa9z2Rfkly3ToEMxqmz9LMdXV-TeEOZFPyMq1Qxx-KVWQvxBpnEwOhWN-mp7IXcQNH8QKYM21Sqj8dmZhu9VmdLw6CtV2AIwbS39m-b4PvMHnL4t6VoB8rd3VsO8x8h6Kx0fT9GHtqSgpQvLdXbkse1YNgqMP49KiifTVB_IhDPf2Ckw',
    'https://lh3.googleusercontent.com/aida-public/AB6AXuAIuhN9Yh6bPcEF328Xbqk89l3QSAFYWt-y713Ed1lif2gHqu3tNNz8J6SuAQWV1BzRoqP1I0t2MeGWV_Mr9VKlNUZlwLEUCevUUvreGBtTFH9c3zR6tgXSg_1AEEiM8ae0yvnAOi20ZeUgUXN7xSfByEGAyu7sLCkrEbVwpKiS7GDYDjD8aYF59DRsESlXjmvOeVGmRy5IM927m9GE9PaSJe7FWK_PKpDh3PppN_kLCAFO2oNhE626qtOrO4pfQ4YvbD7DSITggis',
    'https://lh3.googleusercontent.com/aida-public/AB6AXuCX_-CvBC3pTz83vnWx5oNcJZFV9Hn1kFTO_PLcM-Y_ElR4XvgYnI3evOCbOCCHre0KEw5LaNj9iSJCE2jQG2Cq4iewLbbDRioERaJuIQquqQQzr32uP0dS58l_gnz2-Lu5Dv2DgDz1AkduSndlo3RocJ3vEiGy22-xdo8DQC0l-nMjncVXVe1paknkZuCd8cxU_7AsilwE5F9zVjbymEKqTdc62Cpdfi69V6dWwCTXrI6Fyg5RBuEi0Uo9VvPqbFmHql1_nHIS9Gc',
  ];

  const timelineSteps = $derived.by(() => {
    const startedPhases = timelinePhases.filter((phase) => phase.startedAt);
    const latestStartedPhase = startedPhases.length > 0 ? startedPhases[startedPhases.length - 1].phase : 0;

    return timelinePhases.map((phase) => {
      let status: TimelineStatus = 'pending';
      if (completeEvent) {
        status = 'complete';
      } else if (phase.startedAt) {
        status = phase.phase === latestStartedPhase ? 'active' : 'complete';
      }

      return {
        ...phase,
        status,
        message: progressMessages[phase.phase] ?? buildPhaseMessage(phase.phase),
        logs: phaseLogs[phase.phase] ?? [],
        latestLog: phaseLogs[phase.phase]?.at(-1) ?? null,
      };
    });
  });

  const sourceStats = $derived.by(() => {
    const done = sourceStatuses.filter((item) => item.status === 'done').length;
    const failed = sourceStatuses.filter((item) => item.status !== 'done').length;
    return { total: sourceStatuses.length, done, failed };
  });

  const summaryRiskSignalCount = $derived.by(() =>
    summaryResults.reduce((total, item) => total + item.result.risk_signals.length, 0)
  );

  const extractionRiskSignalCount = $derived.by(() =>
    extractionResults.reduce((total, item) => total + item.result.risk_signals.length, 0)
  );
  const linkedSubjects = $derived.by(() => {
    const graph = linkedSubjectsGraph;
    if (!graph) return [];
    return graph.nodes.filter((node) => node.id !== graph.root_subject_id);
  });

  const uniquePhoneMentions = $derived.by(() =>
    Array.from(new Set(summaryResults.flatMap((item) => item.result.phone_mentions)))
  );

  const uniqueRelatedNumbers = $derived.by(() =>
    Array.from(new Set(extractionResults.flatMap((item) => item.result.related_numbers)))
  );

  const investigationProgress = $derived.by(() => {
    if (completeEvent) return Math.round(completeEvent.confidence * 100);
    const startedCount = timelinePhases.filter((phase) => phase.startedAt).length;
    const progress = startedCount * 18 + (summaryResults.length > 0 ? 6 : 0) + (detectiveNarrative.trim() ? 8 : 0);
    return Math.min(95, Math.max(8, progress));
  });

  const conclusionMeterPercent = $derived.by(() => {
    if (!completeEvent) return investigationProgress;
    return Math.round(completeEvent.confidence * 100);
  });

  const riskCard = $derived.by(() => {
    if (completeEvent) {
      return riskPresentation[completeEvent.risk_level];
    }

    return {
      title: investigationError ? 'Luồng điều tra bị gián đoạn' : 'Đang tổng hợp dữ liệu',
      description: investigationError
        ? investigationError
        : 'Hệ thống đang ghép các tín hiệu từ nguồn quét, nội dung liên quan và phần kết luận AI.',
      panel: investigationError ? 'bg-red-50/70 border-red-200' : 'bg-blue-50/70 border-blue-200',
      iconWrap: investigationError ? 'bg-red-100 text-red-600' : 'bg-blue-100 text-blue-600',
      icon: investigationError ? 'fa-solid fa-plug-circle-xmark' : 'fa-solid fa-spinner animate-spin',
      bar: investigationError ? 'bg-red-500' : 'bg-blue-500',
    };
  });

  const summaryItems = $derived.by(() => {
    const items: { color: string; text: string }[] = [];

    if (sourceStats.total > 0) {
      items.push({
        color: sourceStats.failed > 0 ? 'bg-amber-500' : 'bg-blue-500',
        text: `Đã nhận phản hồi từ ${sourceStats.total} nguồn, ${sourceStats.done} nguồn xử lý thành công.`,
      });
    }

    if (historicalContext) {
      items.push({
        color: 'bg-indigo-500',
        text: `Kho lưu trữ đã có ${historicalContext.investigation_count} lượt tra cứu trước đó, ${historicalContext.approved_reports} báo cáo đã duyệt và ${historicalContext.linked_subjects_count} đối tượng liên kết.`,
      });
    }

    if (summaryResults.length > 0) {
      items.push({
        color: summaryRiskSignalCount > 0 ? 'bg-red-500' : 'bg-emerald-500',
        text: `Đã tóm tắt ${summaryResults.length} nguồn và ghi nhận ${summaryRiskSignalCount} tín hiệu rủi ro.`,
      });
    }

    if (urlAssessment) {
      items.push({
        color: 'bg-amber-500',
        text: `Đã chọn ${urlAssessment.selected}/${urlAssessment.total} trang liên quan để xem xét sâu hơn.`,
      });
    }

    if (extractionResults.length > 0) {
      items.push({
        color: extractionRiskSignalCount > 0 ? 'bg-amber-500' : 'bg-blue-500',
        text: `Đã phân tích ${extractionResults.length} trang liên quan, tìm thấy ${uniqueRelatedNumbers.length} số liên quan và ${extractionRiskSignalCount} tín hiệu rủi ro bổ sung.`,
      });
    }

    if (uniquePhoneMentions.length > 0) {
      items.push({
        color: 'bg-sky-500',
        text: `Có ${uniquePhoneMentions.length} số được nhắc tới trong các nguồn đã tóm tắt.`,
      });
    }

    if (completeEvent) {
      items.push({
        color: 'bg-emerald-500',
        text: `Hoàn tất sau ${formatDuration(completeEvent.duration_ms)} với ${completeEvent.sources_analyzed} nguồn được phân tích.`,
      });
    } else if (detectiveNarrative.trim()) {
      items.push({
        color: 'bg-indigo-500',
        text: 'Điều tra viên AI đã bắt đầu viết phần nhận định và kết luận từ bằng chứng thu thập được.',
      });
    }

    if (items.length === 0) {
      items.push({
        color: 'bg-slate-400',
        text: 'Chưa có phát hiện cụ thể. Hệ thống đang chờ các nguồn đầu tiên trả dữ liệu.',
      });
    }

    return items.slice(0, 5);
  });

  const cleanedDetectiveNarrative = $derived.by(() => normalizeDetectiveNarrative(detectiveNarrativeVisible));

  const formattedDetectiveNarrativeHtml = $derived.by(() =>
    cleanedDetectiveNarrative ? sanitizeReportMarkdown(cleanedDetectiveNarrative) : ''
  );
  const formattedProcessNarrativeHtml = $derived.by(() =>
    processNarrativeVisible ? sanitizeMarkdown(processNarrativeVisible) : ''
  );

  const hasDetectiveNarrative = $derived(cleanedDetectiveNarrative.length > 0);
  const reportTabReady = $derived(Boolean(completeEvent));
  const activeTimelineStep = $derived.by(
    () => timelineSteps.find((step) => step.status === 'active') ?? null
  );
  function createInitialTimeline() {
    return baseTimelinePhases.map((phase) => ({ ...phase }));
  }

  function formatClock(date = new Date()) {
    return new Intl.DateTimeFormat('vi-VN', {
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    }).format(date);
  }

  function formatDuration(durationMs: number) {
    if (durationMs < 1000) return `${durationMs}ms`;
    const seconds = Math.round(durationMs / 100) / 10;
    return `${seconds}s`;
  }

  function formatDateLabel(value: string) {
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return new Intl.DateTimeFormat('vi-VN', {
      day: '2-digit',
      month: '2-digit',
      year: 'numeric',
    }).format(date);
  }

  function confidencePercentText(confidence: number) {
    return `${Math.round(confidence * 100)}%`;
  }

  function confidenceDescription(confidence: number) {
    const percent = confidencePercentText(confidence);
    if (confidence >= 0.85) return `Hệ thống khá chắc chắn về nhận định này: ${percent}.`;
    if (confidence >= 0.65) return `Hệ thống có độ chắc chắn tương đối tốt về nhận định này: ${percent}.`;
    if (confidence >= 0.45) return `Nhận định này ở mức tham khảo, độ chắc chắn khoảng ${percent}.`;
    return `Dữ liệu còn hạn chế, độ chắc chắn hiện khoảng ${percent}.`;
  }

  function formatRiskLevelLabel(riskLevel: RiskLevel) {
    if (riskLevel === 'critical') return 'rất cao';
    if (riskLevel === 'high') return 'cao';
    if (riskLevel === 'medium') return 'trung bình';
    if (riskLevel === 'low') return 'thấp';
    return 'chưa rõ';
  }

  function resolveTab(type: QueryType): TabKey {
    if (type === 'bank') return 'bank';
    if (type === 'url') return 'url';
    return 'phone';
  }

  function buildPhaseMessage(phaseNumber: number) {
    if (phaseNumber === 1) {
      if (sourceStats.total > 0) {
        return `Đã nhận phản hồi từ ${sourceStats.total} nguồn, ${sourceStats.done} nguồn thành công.`;
      }
      return currentQuery
        ? `Đang kết nối tới các nguồn để rà soát ${currentQuery}...`
        : 'Đang kết nối tới các nguồn để bắt đầu điều tra.';
    }

    if (phaseNumber === 2) {
      if (summaryResults.length > 0) {
        return `Đã tóm tắt ${summaryResults.length} nguồn và gom ${summaryRiskSignalCount} tín hiệu rủi ro.`;
      }
      return 'AI đang tóm tắt và chuẩn hóa dữ kiện từ từng nguồn.';
    }

    if (phaseNumber === 3) {
      if (urlAssessment) {
        return `Đã chọn ${urlAssessment.selected}/${urlAssessment.total} trang liên quan để xem xét sâu hơn.`;
      }
      return 'AI đang sàng lọc các trang có giá trị điều tra sâu.';
    }

    if (phaseNumber === 4) {
      const totalTargets = timelinePhases.find((phase) => phase.phase === 4)?.totalSources;
      if (extractionResults.length > 0) {
        return `Đã phân tích ${extractionResults.length}${totalTargets ? `/${totalTargets}` : ''} trang liên quan và bóc tách các dấu hiệu đáng chú ý.`;
      }
      return 'Đang bóc tách các dấu hiệu rủi ro và thông tin liên quan từ từng trang.';
    }

    if (detectiveNarrative.trim()) {
      return 'Điều tra viên AI đang ghép các bằng chứng để viết nhận định cuối cùng.';
    }
    return 'Đang tổng hợp dữ liệu và xây dựng kết luận điều tra.';
  }

  function describeLivePhaseStatus(phaseNumber: number, message: string) {
    const normalized = normalizeLogText(message);
    if (!normalized) return '';

    if (phaseNumber === 2) {
      return 'Hệ thống đang đọc và chắt lọc thông tin từ các nguồn vừa tìm thấy.';
    }

    if (phaseNumber === 3) {
      return 'Hệ thống đang chọn ra những trang liên quan đáng xem xét nhất.';
    }

    if (phaseNumber === 4) {
      return 'Hệ thống đang mở và phân tích nội dung chi tiết từ các trang liên quan.';
    }

    if (phaseNumber === 5) {
      return 'Hệ thống đang ghép các bằng chứng để viết ra nhận định cuối cùng.';
    }

    return normalized;
  }

  function normalizeDetectiveNarrative(value: string) {
    return value
      .replace(/^\s*(RISK_LEVEL|CONFIDENCE)\s*:\s*.+$/gim, '')
      .replace(/^\*\*(Kết luận|Khuyến nghị):\*\*\s*(.+)?$/gim, (_, title: string, rest: string | undefined) => {
        return rest?.trim() ? `## ${title}\n${rest.trim()}` : `## ${title}`;
      })
      .replace(/\n{3,}/g, '\n\n')
      .trim();
  }

  function appendPhaseLog(phase: number, text: string, tone: PhaseLogTone = 'info', replaceKey?: string) {
    const trimmed = text.trim();
    if (!trimmed) return;
    const existing = phaseLogs[phase] ?? [];
    if (replaceKey) {
      const index = existing.findIndex((entry) => entry.replaceKey === replaceKey);
      if (index !== -1) {
        const updated = [...existing];
        updated[index] = { ...updated[index], text: trimmed, tone };
        phaseLogs = { ...phaseLogs, [phase]: updated };
        return;
      }
    }
    const last = existing.at(-1);
    if (last?.text === trimmed) return;
    phaseLogSequence += 1;
    phaseLogs = {
      ...phaseLogs,
      [phase]: [
        ...existing,
        {
          id: phaseLogSequence,
          text: trimmed,
          timestamp: formatClock(),
          tone,
          replaceKey,
        },
      ],
    };
  }

  function normalizeLogText(value: string) {
    return value.replace(/\s+/g, ' ').trim();
  }

  function formatDisplayUrl(url: string) {
    try {
      const parsed = new URL(url);
      return `${parsed.hostname}${parsed.pathname}${parsed.search}`;
    } catch {
      return url;
    }
  }

  function toMarkdownLink(url: string) {
    return `[${formatDisplayUrl(url)}](<${url}>)`;
  }

  function describeSourceStatus(event: SourceStatusEvent) {
    if (event.message) {
      return `**${event.source}.** ${event.message}`;
    }
    if (event.status === 'searching') {
      return `**${event.source}.** Đang tìm kiếm...`;
    }
    if (event.status === 'done') {
      return event.found > 0
        ? `**${event.source}.** Tìm thấy ${event.found} kết quả liên quan.`
        : `**${event.source}.** Đã kiểm tra nhưng chưa thấy kết quả cụ thể.`;
    }
    return `**${event.source}.** Không lấy được dữ liệu hoặc nguồn phản hồi lỗi.`;
  }

  function describeSummaryResult(event: SummaryResultEvent) {
    const summary = normalizeLogText(event.result.summary || event.result.key_facts.join('. '));
    const riskCount = event.result.risk_signals.length;
    const prefix = riskCount > 0
      ? `**${event.source}.** Ghi nhận ${riskCount} tín hiệu rủi ro.`
      : `**${event.source}.** Đã tóm tắt nguồn.`;
    const lines = [`${prefix} ${summary}`];

    if (event.result.risk_signals.length > 0) {
      lines.push(
        [
          `Tín hiệu nổi bật từ ${event.source}:`,
          ...event.result.risk_signals.map((signal) => `- ${normalizeLogText(signal)}`),
        ].join('\n')
      );
    }

    return lines.join('\n\n');
  }

  function describeUrlAssessment(event: UrlAssessmentEvent) {
    const selections = event.urls.map((item, index) =>
      `**Ưu tiên ${index + 1}.** ${toMarkdownLink(item.url)}`
    );
    return [
      `**Đã chọn ${event.selected}/${event.total} trang liên quan để xem xét sâu hơn.**`,
      ...selections,
    ];
  }

  function describeExtractionResult(event: ExtractionResultEvent) {
    const summary = normalizeLogText(event.result.summary || event.result.risk_signals.join('. '));
    const lines = [`**${toMarkdownLink(event.url)}.** ${summary}`];

    if (event.result.related_numbers.length > 0) {
      lines.push(
        [
          'Số liên quan được bóc tách:',
          ...event.result.related_numbers.map((number) => `- ${number}`),
        ].join('\n')
      );
    }

    if (event.result.risk_signals.length > 0) {
      lines.push(
        [
          'Dấu hiệu rủi ro từ nội dung chi tiết:',
          ...event.result.risk_signals.map((signal) => `- ${normalizeLogText(signal)}`),
        ].join('\n')
      );
    }

    return lines.join('\n\n');
  }

  function resetInvestigationState() {
    investigationPanelTab = 'process';
    currentInvestigationId = '';
    linkedSubjectsAbortController?.abort();
    linkedSubjectsAbortController = null;
    if (reportStreamStartTimer) {
      clearTimeout(reportStreamStartTimer);
      reportStreamStartTimer = null;
    }
    clearProcessNarrativeTimer();
    processNarrativeTarget = '';
    processNarrativeVisible = '';
    streamedProcessPhaseHeadings = new Set();
    streamedProcessEntries = new Set();
    processNarrativeReplaceMap = new Map();
    sourceReplaceAnimations.forEach((timer) => clearTimeout(timer));
    sourceReplaceAnimations = new Map();
    timelinePhases = createInitialTimeline();
    sourceStatuses = [];
    summaryResults = [];
    urlAssessment = null;
    extractionResults = [];
    detectiveNarrative = '';
    detectiveNarrativeVisible = '';
    reportNarrativePrimed = false;
    phaseLogs = {};
    progressMessages = {};
    completeEvent = null;
    investigationError = null;
    reportStreamError = null;
    historicalContext = null;
    linkedSubjectsGraph = null;
    linkedSubjectsLoading = false;
    linkedSubjectsError = null;
    reportDescription = '';
    reportCategory = 'scam';
    reportSubmitting = false;
    reportSubmitMessage = null;
    reportSubmitError = null;
    phaseLogSequence = 0;
  }

  function closeInvestigation() {
    investigationHandle?.close();
    investigationHandle = null;
    reportHandle?.close();
    reportHandle = null;
    linkedSubjectsAbortController?.abort();
    linkedSubjectsAbortController = null;
    if (reportStreamStartTimer) {
      clearTimeout(reportStreamStartTimer);
      reportStreamStartTimer = null;
    }
    clearReportNarrativeTimer();
  }

  function clearProcessNarrativeTimer() {
    if (!processNarrativeTimer) return;
    clearTimeout(processNarrativeTimer);
    processNarrativeTimer = null;
  }

  function clearReportNarrativeTimer() {
    if (!reportNarrativeTimer) return;
    clearTimeout(reportNarrativeTimer);
    reportNarrativeTimer = null;
  }

  function getProcessNarrativeDelay(currentChar: string, nextChar: string) {
    if (currentChar === '\n' && nextChar === '\n') return 90;
    if (currentChar === '\n') return 40;
    if (/[.!?:;]/.test(currentChar)) return 26;
    if (/\s/.test(currentChar)) return 12;
    return 9;
  }

  function getReportNarrativeDelay(currentChar: string, nextChar: string) {
    if (currentChar === '\n' && nextChar === '\n') return 110;
    if (currentChar === '\n') return 52;
    if (/[.!?:;]/.test(currentChar)) return 30;
    if (/\s/.test(currentChar)) return 12;
    return 9;
  }

  function scheduleProcessNarrativeTyping(initialDelay = 0) {
    if (processNarrativeTimer) return;

    const typeNextCharacter = () => {
      processNarrativeTimer = null;

      const target = processNarrativeTarget;
      if (!target) {
        processNarrativeVisible = '';
        return;
      }

      if (!target.startsWith(processNarrativeVisible)) {
        processNarrativeVisible = '';
      }

      if (processNarrativeVisible.length >= target.length) {
        processNarrativeVisible = target;
        return;
      }

      const nextIndex = processNarrativeVisible.length + 1;
      const currentChar = target[nextIndex - 1] ?? '';
      const nextChar = target[nextIndex] ?? '';

      processNarrativeVisible = target.slice(0, nextIndex);
      processNarrativeTimer = setTimeout(typeNextCharacter, getProcessNarrativeDelay(currentChar, nextChar));
    };

    processNarrativeTimer = setTimeout(typeNextCharacter, initialDelay);
  }

  function scheduleReportNarrativeTyping(initialDelay = 0) {
    if (reportNarrativeTimer) return;

    const typeNextCharacter = () => {
      reportNarrativeTimer = null;

      const target = detectiveNarrative;
      if (!target) {
        detectiveNarrativeVisible = '';
        return;
      }

      if (!target.startsWith(detectiveNarrativeVisible)) {
        detectiveNarrativeVisible = '';
      }

      if (detectiveNarrativeVisible.length >= target.length) {
        detectiveNarrativeVisible = target;
        return;
      }

      const nextIndex = detectiveNarrativeVisible.length + 1;
      const currentChar = target[nextIndex - 1] ?? '';
      const nextChar = target[nextIndex] ?? '';

      detectiveNarrativeVisible = target.slice(0, nextIndex);
      reportNarrativeTimer = setTimeout(typeNextCharacter, getReportNarrativeDelay(currentChar, nextChar));
    };

    reportNarrativeTimer = setTimeout(typeNextCharacter, initialDelay);
  }

  function ensureProcessPhaseHeading(phase: number, label: string) {
    if (streamedProcessPhaseHeadings.has(phase)) return;
    streamedProcessPhaseHeadings.add(phase);
    appendProcessNarrativeMarkdown(`## ${label}`);
  }

  let sourceReplaceAnimations = new Map<string, ReturnType<typeof setTimeout>>();

  function animateNarrativeReplace(oldText: string, newText: string, replaceKey: string) {
    // Cancel any existing animation for this key
    const existingTimer = sourceReplaceAnimations.get(replaceKey);
    if (existingTimer) clearTimeout(existingTimer);

    // Find common prefix (e.g. "**CheckScam.** ")
    let prefixLen = 0;
    while (prefixLen < oldText.length && prefixLen < newText.length && oldText[prefixLen] === newText[prefixLen]) prefixLen++;
    const prefix = oldText.slice(0, prefixLen);
    const oldSuffix = oldText.slice(prefixLen);
    const newSuffix = newText.slice(prefixLen);

    let currentText = oldText;
    let deleteIdx = oldSuffix.length;
    let typeIdx = 0;

    const DELETE_SPEED = 12;
    const TYPE_SPEED = 22;
    const DELETE_STEP = 3;

    function replaceInNarratives(from: string, to: string) {
      processNarrativeTarget = processNarrativeTarget.replace(from, to);
      processNarrativeVisible = processNarrativeVisible.replace(from, to);
    }

    function step() {
      if (deleteIdx > 0) {
        deleteIdx = Math.max(0, deleteIdx - DELETE_STEP);
        const next = prefix + oldSuffix.slice(0, deleteIdx);
        replaceInNarratives(currentText, next);
        currentText = next;
        sourceReplaceAnimations.set(replaceKey, setTimeout(step, DELETE_SPEED));
      } else if (typeIdx < newSuffix.length) {
        typeIdx++;
        const next = prefix + newSuffix.slice(0, typeIdx);
        replaceInNarratives(currentText, next);
        currentText = next;
        sourceReplaceAnimations.set(replaceKey, setTimeout(step, TYPE_SPEED));
      } else {
        processNarrativeReplaceMap.set(replaceKey, newText);
        sourceReplaceAnimations.delete(replaceKey);
      }
    }

    step();
  }

  function appendProcessNarrativeMarkdown(markdown: string, dedupeKey?: string, replaceKey?: string) {
    const trimmed = markdown.trim();
    if (!trimmed) return;

    if (replaceKey && processNarrativeTarget) {
      const existing = processNarrativeReplaceMap.get(replaceKey);
      if (existing !== undefined) {
        // Check if old text is already visible — animate it
        if (processNarrativeVisible.includes(existing)) {
          animateNarrativeReplace(existing, trimmed, replaceKey);
        } else {
          // Not yet visible — just replace in target, typewriter will type the new text
          processNarrativeTarget = processNarrativeTarget.replace(existing, trimmed);
          processNarrativeReplaceMap.set(replaceKey, trimmed);
        }
        return;
      }
      processNarrativeReplaceMap.set(replaceKey, trimmed);
    }

    const normalizedKey = dedupeKey ?? normalizeLogText(trimmed);
    if (normalizedKey && streamedProcessEntries.has(normalizedKey)) return;
    if (normalizedKey) {
      streamedProcessEntries.add(normalizedKey);
    }

    processNarrativeTarget = processNarrativeTarget
      ? `${processNarrativeTarget}\n\n${trimmed}`
      : trimmed;
  }

  function appendProcessPhaseNarrative(phase: number, label: string, markdown: string, dedupeKey?: string, replaceKey?: string) {
    ensureProcessPhaseHeading(phase, label);
    appendProcessNarrativeMarkdown(markdown, dedupeKey ? `${phase}:${dedupeKey}` : `${phase}:${normalizeLogText(markdown)}`, replaceKey ? `${phase}:${replaceKey}` : undefined);
  }

  function revealCurrentProcessNarrative() {
    clearProcessNarrativeTimer();
    processNarrativeVisible = processNarrativeTarget;
  }

  function upsertTimelinePhase(phaseNumber: number, values: Partial<TimelinePhase>) {
    const index = timelinePhases.findIndex((phase) => phase.phase === phaseNumber);
    if (index === -1) return;
    timelinePhases[index] = { ...timelinePhases[index], ...values };
  }

  function upsertSourceStatus(event: SourceStatusEvent) {
    const index = sourceStatuses.findIndex((item) => item.source === event.source);
    if (index === -1) {
      sourceStatuses = [...sourceStatuses, event];
      return;
    }
    sourceStatuses[index] = event;
  }

  function upsertSummaryResult(event: SummaryResultEvent) {
    const index = summaryResults.findIndex((item) => item.source === event.source);
    if (index === -1) {
      summaryResults = [...summaryResults, event];
      return;
    }
    summaryResults[index] = event;
  }

  function upsertExtractionResult(event: ExtractionResultEvent) {
    const index = extractionResults.findIndex((item) => item.url === event.url);
    if (index === -1) {
      extractionResults = [...extractionResults, event];
      return;
    }
    extractionResults[index] = event;
  }

  function handleReportChunk(event: { chunk: string; replace: boolean }) {
    detectiveNarrative = event.replace ? event.chunk : `${detectiveNarrative}${event.chunk}`;
  }

  function describeHistoricalContext(event: HistoricalContextSseEvent) {
    return `Kho lưu trữ đã có ${event.investigation_count} hồ sơ trước đó, ${event.approved_reports} báo cáo đã duyệt và ${event.linked_subjects_count} đối tượng liên kết cho mục đang tra cứu.`;
  }

  async function loadLinkedSubjects(query: string, type: QueryType) {
    linkedSubjectsAbortController?.abort();
    linkedSubjectsLoading = true;
    linkedSubjectsError = null;

    const requestId = ++linkedSubjectsRequestId;
    const controller = new AbortController();
    linkedSubjectsAbortController = controller;

    try {
      const endpoint = buildApiUrl(
        `/api/subjects/${encodeURIComponent(query)}/network?type=${type}&depth=2`,
      );
      for (let attempt = 0; attempt < 4; attempt += 1) {
        const response = await fetch(endpoint, { signal: controller.signal });
        if (response.status === 404) {
          if (attempt < 3) {
            await new Promise((resolve) => setTimeout(resolve, 300 * (attempt + 1)));
            continue;
          }
          if (requestId === linkedSubjectsRequestId) {
            linkedSubjectsGraph = null;
          }
          return;
        }
        if (!response.ok) {
          const body = await response.json().catch(() => ({ error: '' }));
          throw new Error(body.error || 'Không tải được mạng liên kết.');
        }
        const graph = (await response.json()) as NetworkGraph;
        if (graph.nodes.length <= 1 && attempt < 2) {
          await new Promise((resolve) => setTimeout(resolve, 300 * (attempt + 1)));
          continue;
        }
        if (requestId !== linkedSubjectsRequestId) return;
        linkedSubjectsGraph = graph;
        return;
      }
    } catch (error) {
      if (controller.signal.aborted || requestId !== linkedSubjectsRequestId) return;
      linkedSubjectsError = error instanceof Error ? error.message : 'Không tải được mạng liên kết.';
    } finally {
      if (requestId === linkedSubjectsRequestId) {
        linkedSubjectsLoading = false;
      }
      if (linkedSubjectsAbortController === controller) {
        linkedSubjectsAbortController = null;
      }
    }
  }

  async function submitCommunityReport() {
    reportSubmitError = null;
    reportSubmitMessage = null;
    const description = reportDescription.trim();
    if (!description) {
      reportSubmitError = 'Bạn cần nhập mô tả trước khi gửi báo cáo.';
      return;
    }

    reportSubmitting = true;
    try {
      const response = await fetch(buildApiUrl('/api/reports'), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          value: currentQuery,
          subject_type: currentType,
          description,
          category: reportCategory,
        }),
      });
      const body = await response.json().catch(() => ({ error: '' }));
      if (!response.ok) {
        throw new Error(body.error || 'Không gửi được báo cáo.');
      }
      reportDescription = '';
      reportCategory = 'scam';
      reportSubmitMessage = 'Đã gửi báo cáo cộng đồng. Nội dung sẽ chờ quản trị viên duyệt.';
    } catch (error) {
      reportSubmitError = error instanceof Error ? error.message : 'Không gửi được báo cáo.';
    } finally {
      reportSubmitting = false;
    }
  }

  function startReportStream(investigationId: string) {
    if (!investigationId || reportHandle) return;
    reportStreamError = null;

    try {
      reportHandle = createInvestigationReportStream(investigationId, {
        onDetectiveStream: (event) => {
          handleReportChunk(event);
          if (event.done) {
            reportHandle?.close();
            reportHandle = null;
          }
        },
        onError: (event) => {
          reportStreamError = event.message;
          reportHandle?.close();
          reportHandle = null;
        },
      });
    } catch {
      reportStreamError = 'Chưa thể mở báo cáo lúc này.';
    }
  }

  function startInvestigation(query: string, type: QueryType) {
    closeInvestigation();
    resetInvestigationState();

    currentQuery = query;
    currentType = type;
    pageState = 'investigating';

    try {
      investigationHandle = createInvestigation(query, type, {
        onInvestigationStarted: (event: InvestigationStartedEvent) => {
          currentInvestigationId = event.investigation_id;
        },
        onPhaseStart: (event) => {
          upsertTimelinePhase(event.phase, {
            label: event.label || timelinePhases.find((phase) => phase.phase === event.phase)?.label || `Pha ${event.phase}`,
            startedAt: formatClock(),
            totalSources: event.total_sources,
          });
          appendProcessPhaseNarrative(
            event.phase,
            event.label || timelinePhases.find((phase) => phase.phase === event.phase)?.label || `Pha ${event.phase}`,
            `> **Đang xử lý:** ${describeLivePhaseStatus(event.phase, buildPhaseMessage(event.phase))}`,
            'phase-start'
          );
          revealCurrentProcessNarrative();
        },
        onSourceStatus: (event) => {
          upsertSourceStatus(event);
          const description = describeSourceStatus(event);
          const logType = event.status === 'searching' ? 'info' : event.status === 'done' ? 'success' : 'warning';
          const sourceKey = `source:${event.source}`;
          appendPhaseLog(1, description, logType, sourceKey);
          appendProcessPhaseNarrative(1, timelinePhases.find((phase) => phase.phase === 1)?.label || 'Thu thập dữ liệu', description, `source:${event.source}:${event.status}:${event.found}`, sourceKey);
        },
        onProgress: (event) => {
          progressMessages = { ...progressMessages, [event.phase]: event.message };
        },
        onSummaryResult: (event) => {
          upsertSummaryResult(event);
          const description = describeSummaryResult(event);
          appendPhaseLog(2, description, event.result.risk_signals.length > 0 ? 'warning' : 'success');
          appendProcessPhaseNarrative(2, timelinePhases.find((phase) => phase.phase === 2)?.label || 'Tóm tắt nguồn', description, `summary:${event.source}`);
        },
        onUrlAssessment: (event) => {
          urlAssessment = event;
          const description = describeUrlAssessment(event).join('\n\n');
          for (const line of describeUrlAssessment(event)) {
            appendPhaseLog(3, line, line.includes('Đã chọn') ? 'success' : 'info');
          }
          appendProcessPhaseNarrative(3, timelinePhases.find((phase) => phase.phase === 3)?.label || 'Chọn nguồn cần xem kỹ', description, `url-assessment:${event.selected}:${event.total}`);
        },
        onExtractionResult: (event) => {
          upsertExtractionResult(event);
          const description = describeExtractionResult(event);
          appendPhaseLog(4, description, event.result.risk_signals.length > 0 ? 'warning' : 'success');
          appendProcessPhaseNarrative(4, timelinePhases.find((phase) => phase.phase === 4)?.label || 'Phân tích nội dung chi tiết', description, `extraction:${event.url}`);
        },
        onHistoricalContext: (event) => {
          historicalContext = event;
          const description = describeHistoricalContext(event);
          appendPhaseLog(1, description, 'info');
          appendProcessPhaseNarrative(1, timelinePhases.find((phase) => phase.phase === 1)?.label || 'Thu thập dữ liệu', description, `historical:${event.investigation_count}:${event.linked_subjects_count}`);
        },
        onDetectiveStream: (event) => {
          if (!phaseLogs[5]?.length) {
            appendPhaseLog(5, 'Điều tra viên AI đã bắt đầu viết nhận định từ các bằng chứng đã thu thập.', 'success');
            appendProcessPhaseNarrative(5, timelinePhases.find((phase) => phase.phase === 5)?.label || 'Tổng hợp điều tra', 'Điều tra viên AI đã bắt đầu viết nhận định từ các bằng chứng đã thu thập.', 'detective-start');
            revealCurrentProcessNarrative();
          }
          handleReportChunk(event);
        },
        onComplete: (event) => {
          currentInvestigationId = event.investigation_id;
          completeEvent = event;
          const description = `Đã hoàn tất điều tra. Hệ thống đánh giá mức cảnh báo ${formatRiskLevelLabel(event.risk_level)} với độ chắc chắn ${confidencePercentText(event.confidence)}.`;
          appendPhaseLog(5, description, 'success');
          appendProcessPhaseNarrative(5, timelinePhases.find((phase) => phase.phase === 5)?.label || 'Tổng hợp điều tra', description, `complete:${event.risk_level}:${event.confidence}`);
          investigationPanelTab = 'report';
          clearReportNarrativeTimer();
          detectiveNarrativeVisible = '';
          reportNarrativePrimed = true;
          if (!detectiveNarrative.trim()) {
            tick().then(() => {
              reportStreamStartTimer = setTimeout(() => {
                reportStreamStartTimer = null;
                if (currentInvestigationId === event.investigation_id && pageState === 'investigating') {
                  startReportStream(event.investigation_id);
                }
              }, 0);
            });
          }
          void loadLinkedSubjects(query, type);
        },
        onError: (event) => {
          investigationError = event.message;
          if (event.phase) {
            appendPhaseLog(event.phase, event.message, 'warning');
            appendProcessPhaseNarrative(
              event.phase,
              timelinePhases.find((phase) => phase.phase === event.phase)?.label || `Pha ${event.phase}`,
              `> **Cảnh báo:** ${event.message}`,
              `error:${event.phase}:${event.message}`
            );
          }
        },
      });
    } catch {
      investigationError = 'Chưa thể kết nối để cập nhật kết quả.';
    }
  }

  function submitSearch() {
    const trimmed = searchQuery.trim();
    if (!trimmed) return;

    let type: QueryType = 'phone';
    if (selectedTab === 'bank') type = 'bank';
    else if (selectedTab === 'url' || selectedTab === 'other') type = 'url';

    startInvestigation(trimmed, type);
  }

  $effect(() => {
    if (!autoStarted && data.q.trim()) {
      autoStarted = true;
      searchQuery = data.q;
      selectedTab = resolveTab(data.type);
      startInvestigation(data.q, data.type);
    }
  });

  $effect(() => {
    return () => {
      clearProcessNarrativeTimer();
      clearReportNarrativeTimer();
      closeInvestigation();
    };
  });

  $effect(() => {
    const target = processNarrativeTarget;
    const visible = processNarrativeVisible;

    if (!target) {
      clearProcessNarrativeTimer();
      processNarrativeVisible = '';
      return;
    }

    if (completeEvent || investigationError) {
      clearProcessNarrativeTimer();
      processNarrativeVisible = target;
      return;
    }

    if (!target.startsWith(visible)) {
      clearProcessNarrativeTimer();
      processNarrativeVisible = visible ? `${visible}\n\n${target.slice(visible.length).trimStart()}` : target;
      return;
    }

    if (visible.length < target.length) {
      scheduleProcessNarrativeTyping();
      return;
    }

    clearProcessNarrativeTimer();
  });

  $effect(() => {
    const target = detectiveNarrative;
    const visible = detectiveNarrativeVisible;

    if (!target) {
      clearReportNarrativeTimer();
      detectiveNarrativeVisible = '';
      reportNarrativePrimed = false;
      return;
    }

    if (investigationPanelTab !== 'report') {
      clearReportNarrativeTimer();
      return;
    }

    if (!target.startsWith(visible)) {
      clearReportNarrativeTimer();
      detectiveNarrativeVisible = '';
      reportNarrativePrimed = true;
    }

    if (detectiveNarrativeVisible.length < target.length) {
      const initialDelay = reportNarrativePrimed && detectiveNarrativeVisible.length === 0 ? 140 : 0;
      reportNarrativePrimed = false;
      scheduleReportNarrativeTyping(initialDelay);
      return;
    }

    clearReportNarrativeTimer();
  });

  $effect(() => {
    pageState;
    investigationPanelTab;
    processNarrativeVisible;
    formattedDetectiveNarrativeHtml;

    if (pageState !== 'investigating') return;
    const target = investigationPanelTab === 'report' ? reportScrollViewport : processScrollViewport;
    if (!target) return;

    tick().then(() => {
      target.scrollTop = target.scrollHeight;
    });
  });
</script>

{#if pageState === 'home'}
  <div class="anim-0">
    <!-- ==================== HEADER ==================== -->
    <header class="bg-white/80 backdrop-blur-md sticky top-0 z-50 border-b border-gray-100">
      <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
        <div class="flex justify-between items-center h-20">
          <div class="flex items-center flex-shrink-0 cursor-pointer gap-3">
            <img src="/logo.svg" alt="Logo tracuuluadao.vn" class="h-10 w-auto" />
            <span class="font-bold text-xl text-gray-900">tracuuluadao.vn</span>
          </div>

          <nav class="hidden md:flex space-x-8">
            <a class="text-sm font-medium text-gray-700 hover:text-blue-600 nav-link active flex items-center gap-1" href="/">
              Trang chủ <i class="fa-solid fa-chevron-down text-xs"></i>
            </a>
            <button type="button" class="text-sm font-medium text-gray-700 hover:text-blue-600 flex items-center gap-1">
              Hướng dẫn <i class="fa-solid fa-chevron-down text-xs"></i>
            </button>
            <button type="button" class="text-sm font-medium text-gray-700 hover:text-blue-600 flex items-center gap-1">
              Câu hỏi thường gặp <i class="fa-solid fa-chevron-down text-xs"></i>
            </button>
            <button type="button" class="text-sm font-medium text-gray-700 hover:text-blue-600">Blog</button>
          </nav>

          <div class="flex items-center space-x-4">
            <button class="flex items-center gap-2 px-4 py-2 border border-gray-200 rounded-full text-sm font-medium text-gray-700 hover:bg-gray-50 transition">
              <i class="fa-solid fa-shield-halved text-gray-400"></i> Báo cáo của tôi
            </button>
            <button type="button" aria-label="Tài khoản người dùng" class="w-10 h-10 rounded-full border border-gray-200 flex items-center justify-center text-gray-600 hover:bg-gray-50 transition">
              <i class="fa-regular fa-user"></i>
            </button>
          </div>
        </div>
      </div>
    </header>

    <!-- ==================== HERO ==================== -->
    <section class="relative bg-gradient-to-br from-[#f4f7ff] to-white pt-10 pb-32 overflow-hidden">
      <div class="absolute top-0 right-0 -mr-20 -mt-20 w-96 h-96 rounded-full bg-blue-100 blur-3xl opacity-50"></div>
      <div class="absolute bottom-0 left-0 -ml-20 -mb-20 w-80 h-80 rounded-full bg-purple-100 blur-3xl opacity-50"></div>

      <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 relative z-10">
        <div class="grid grid-cols-1 lg:grid-cols-[65%_35%] gap-12 items-center">
          <!-- Left Column -->
          <div class="w-full">
            <div class="inline-flex items-center gap-2 px-4 py-2 rounded-full bg-gradient-to-r from-blue-50 to-purple-50 border border-blue-100 text-blue-700 text-xs font-semibold mb-6 shadow-sm">
              <i class="fa-solid fa-robot text-blue-600"></i> NỀN TẢNG TRA CỨU & PHÂN TÍCH THÔNG TIN THÔNG MINH
            </div>

            <h1 class="text-5xl lg:text-6xl font-extrabold text-gray-900 leading-tight mb-8">
              <span class="gradient-text">AI</span> tra cứu<br/>thông tin lừa đảo
            </h1>

            <!-- Search Widget -->
            <div class="bg-white rounded-2xl shadow-xl p-6 border border-gray-100 relative z-20">
              <!-- Tabs -->
              <div class="flex space-x-6 border-b border-gray-100 mb-6 pb-2 overflow-x-auto no-scrollbar">
                {#each tabItems as tab}
                  <button
                    type="button"
                    onclick={() => selectedTab = tab.key}
                    class="{selectedTab === tab.key ? 'text-blue-600 font-semibold border-b-2 border-blue-600' : 'text-gray-500 hover:text-gray-900'} pb-2 whitespace-nowrap flex items-center gap-2"
                  >
                    <i class="fa-solid {tab.icon}"></i> {tab.label}
                  </button>
                {/each}
              </div>

              <!-- Input -->
              <div class="relative mb-4">
                <input
                  type="text"
                  bind:value={searchQuery}
                  placeholder={tabPlaceholders[selectedTab]}
                  class="w-full px-4 py-4 bg-gray-50 border border-gray-200 rounded-xl focus:ring-2 focus:ring-blue-500 focus:border-blue-500 text-gray-900 pr-10"
                  onkeydown={(e: KeyboardEvent) => e.key === 'Enter' && submitSearch()}
                />
                {#if searchQuery}
                  <button type="button" aria-label="Xóa nội dung tìm kiếm" onclick={() => searchQuery = ''} class="absolute right-4 top-1/2 transform -translate-y-1/2 text-gray-400 hover:text-gray-600">
                    <i class="fa-solid fa-xmark"></i>
                  </button>
                {/if}
              </div>

              <!-- Search Button -->
              <button
                onclick={submitSearch}
                class="w-full gradient-btn text-white font-bold py-4 rounded-xl shadow-md transition flex justify-center items-center gap-2 text-lg"
              >
                <i class="fa-solid fa-magnifying-glass"></i> Bắt đầu tra cứu ngay
              </button>

              <div class="mt-4 text-center text-sm text-gray-500 flex items-center justify-center gap-2">
                <i class="fa-solid fa-lock text-gray-400"></i> Chúng tôi cam kết bảo mật tuyệt đối thông tin của bạn.
              </div>
            </div>
          </div>

          <!-- Right Column -->
          <div class="relative lg:h-[600px] flex flex-col justify-center items-center sm:-mt-1 lg:-mt-3">
            <!-- Main Illustration -->
            <div class="relative w-full max-w-md aspect-square mx-auto">
              <div class="absolute inset-0 bg-blue-100 rounded-full blur-3xl opacity-60"></div>

              <!-- Detective robot GIF -->
              <div class="absolute inset-0 flex items-center justify-center z-10">
                <img
                  src="/robot-homepage.gif"
                  alt="Robot thám tử"
                  class="relative z-10 h-auto w-full max-w-[390px] object-contain drop-shadow-[0_24px_48px_rgba(37,99,235,0.18)]"
                />
              </div>

              <!-- Floating Elements -->
              <div class="absolute top-10 left-10 bg-white p-3 rounded-2xl shadow-lg animate-bounce z-20">
                <i class="fa-solid fa-phone text-blue-500 text-xl"></i>
              </div>
              <div class="absolute top-20 right-20 bg-white p-3 rounded-2xl shadow-lg animate-pulse z-20">
                <i class="fa-solid fa-globe text-blue-500 text-xl"></i>
              </div>
              <div class="absolute bottom-32 left-10 bg-white p-3 rounded-2xl shadow-lg z-20">
                <i class="fa-solid fa-user text-blue-500 text-xl"></i>
              </div>
              <div class="absolute bottom-40 right-10 bg-white p-3 rounded-2xl shadow-lg z-20">
                <i class="fa-regular fa-credit-card text-blue-500 text-xl"></i>
              </div>

              <svg class="absolute inset-0 w-full h-full text-gray-200 z-0" viewBox="0 0 100 100">
                <path d="M20,20 Q50,50 80,80" fill="none" stroke="currentColor" stroke-dasharray="2,2" stroke-width="0.5"></path>
                <path d="M80,20 Q50,50 20,80" fill="none" stroke="currentColor" stroke-dasharray="2,2" stroke-width="0.5"></path>
              </svg>
            </div>

            <!-- Trust Indicator Card — Apple Glass Style -->
            <div class="w-full max-w-lg relative z-30 -mt-1">
              <!-- Outer glow -->
              <div class="absolute -inset-1 rounded-3xl blur-xl opacity-40"
                   style="background: linear-gradient(135deg, rgba(255,255,255,0.6), rgba(59,130,246,0.15), rgba(255,255,255,0.4))">
              </div>

              <div class="relative rounded-2xl p-6 flex items-center gap-5 overflow-hidden"
                   style="
                     background: rgba(255,255,255,0.18);
                     backdrop-filter: blur(40px) saturate(200%);
                     -webkit-backdrop-filter: blur(40px) saturate(200%);
                     border: 1px solid rgba(255,255,255,0.45);
                     box-shadow:
                       0 0 0 0.5px rgba(255,255,255,0.3) inset,
                       0 2px 12px rgba(0,0,0,0.06),
                       0 16px 48px rgba(59,130,246,0.08);
                   ">

                <!-- Inner top highlight (Apple shimmer) -->
                <div class="absolute top-0 left-0 right-0 h-px"
                     style="background: linear-gradient(90deg, transparent, rgba(255,255,255,0.8), transparent)">
                </div>

                <!-- Icon -->
                <div class="shrink-0 w-12 h-12 rounded-2xl flex items-center justify-center"
                     style="background: rgba(59,130,246,0.12); border: 1px solid rgba(59,130,246,0.2)">
                  <i class="fa-solid fa-users text-blue-600" style="font-size:20px"></i>
                </div>

                <!-- Text -->
                <div class="flex-1 min-w-0">
                  <h4 class="font-bold text-gray-900 text-base sm:text-lg leading-tight">Hơn 150.000+ người dùng</h4>
                  <p class="text-sm text-gray-500 mt-1 leading-relaxed">đã tin tưởng để bảo vệ bản thân khỏi rủi ro lừa đảo.</p>

                  <!-- Avatar stack -->
                  <div class="flex items-center mt-3">
                    <div class="flex -space-x-1.5">
                      {#each avatarUrls as url}
                        <img src={url} alt="User"
                             class="w-7 h-7 rounded-full border border-white/80 object-cover"
                             style="box-shadow: 0 1px 3px rgba(0,0,0,0.15)" />
                      {/each}
                      <div class="shrink-0 w-7 h-7 rounded-full border border-white/80 flex items-center justify-center text-[9px] leading-none font-bold text-gray-600"
                           style="background: rgba(255,255,255,0.9); box-shadow: 0 1px 3px rgba(0,0,0,0.12)">
                        +99K
                      </div>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>

    <!-- ==================== FEATURES ==================== -->
    <section class="py-16 bg-white">
      <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
        <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-8">
          {#each featureCards as feature}
            <div class="bg-gray-50 rounded-2xl p-6 border border-gray-100 hover:shadow-md transition">
              <div class="w-12 h-12 bg-{feature.color}-100 rounded-xl flex items-center justify-center mb-4">
                <i class="fa-solid {feature.icon} text-{feature.color}-600 text-xl"></i>
              </div>
              <h3 class="text-lg font-bold text-gray-900 mb-2">{feature.title}</h3>
              <p class="text-gray-600 text-sm mb-4">{feature.desc}</p>
              <span class="inline-flex items-center gap-1 px-2.5 py-1 rounded-full bg-{feature.color}-50 text-{feature.color}-600 text-xs font-medium">
                <i class="fa-solid fa-sparkles text-[10px]"></i> {feature.badge}
              </span>
            </div>
          {/each}
        </div>
      </div>
    </section>

    <!-- ==================== STATS ==================== -->
    <section class="py-12 bg-white pb-24">
      <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
        <div class="bg-gray-50 rounded-3xl p-8 border border-gray-100">
          <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-8 divide-y md:divide-y-0 md:divide-x divide-gray-200">
            {#each stats as stat}
              <div class="flex items-center gap-4 pt-4 md:pt-0 md:px-4">
                <div class="w-12 h-12 bg-{stat.color}-100 rounded-full flex items-center justify-center text-{stat.color}-600 text-xl">
                  <i class="{stat.regular ? 'fa-regular' : 'fa-solid'} {stat.icon}"></i>
                </div>
                <div>
                  <div class="text-2xl font-bold text-gray-900">{stat.value}</div>
                  <div class="text-sm text-gray-600">{stat.label}</div>
                </div>
              </div>
            {/each}
          </div>
        </div>
      </div>
    </section>

    <!-- ==================== FOOTER ==================== -->
    <footer class="bg-gray-50 border-t border-gray-200 py-16">
      <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
        <div class="grid grid-cols-1 md:grid-cols-4 lg:grid-cols-5 gap-12">
          <!-- Brand Col -->
          <div class="lg:col-span-2">
            <div class="flex items-center flex-shrink-0 mb-4 gap-3">
              <img src="/logo.svg" alt="Logo tracuuluadao.vn" class="h-10 w-auto" />
              <span class="font-bold text-xl text-gray-900">tracuuluadao.vn</span>
            </div>
            <p class="text-sm text-gray-600 max-w-xs mb-6">
              Nền tảng tra cứu và phân tích thông tin đáng tin cậy, bảo vệ bạn khỏi rủi ro lừa đảo.
            </p>
          </div>
          <!-- Links Col 1 -->
          <div>
            <h4 class="font-bold text-gray-900 mb-4">Về chúng tôi</h4>
            <ul class="space-y-3">
              <li><button type="button" class="text-sm text-gray-600 hover:text-blue-600">Giới thiệu</button></li>
              <li><button type="button" class="text-sm text-gray-600 hover:text-blue-600">Điều khoản sử dụng</button></li>
              <li><button type="button" class="text-sm text-gray-600 hover:text-blue-600">Chính sách bảo mật</button></li>
            </ul>
          </div>
          <!-- Links Col 2 -->
          <div>
            <h4 class="font-bold text-gray-900 mb-4">Hỗ trợ</h4>
            <ul class="space-y-3">
              <li><button type="button" class="text-sm text-gray-600 hover:text-blue-600">Hướng dẫn sử dụng</button></li>
              <li><button type="button" class="text-sm text-gray-600 hover:text-blue-600">Câu hỏi thường gặp</button></li>
              <li><button type="button" class="text-sm text-gray-600 hover:text-blue-600">Liên hệ hỗ trợ</button></li>
            </ul>
          </div>
          <!-- Social Col -->
          <div>
            <h4 class="font-bold text-gray-900 mb-4">Kết nối với chúng tôi</h4>
            <div class="flex space-x-4 mb-6">
              <button type="button" aria-label="Facebook" class="w-10 h-10 rounded-full bg-gray-200 flex items-center justify-center text-gray-600 hover:bg-blue-600 hover:text-white transition">
                <i class="fa-brands fa-facebook-f"></i>
              </button>
              <button type="button" aria-label="Twitter" class="w-10 h-10 rounded-full bg-gray-200 flex items-center justify-center text-gray-600 hover:bg-blue-400 hover:text-white transition">
                <i class="fa-brands fa-twitter"></i>
              </button>
              <button type="button" aria-label="Email" class="w-10 h-10 rounded-full bg-gray-200 flex items-center justify-center text-gray-600 hover:bg-red-500 hover:text-white transition">
                <i class="fa-regular fa-envelope"></i>
              </button>
            </div>
            <div class="flex items-start gap-3 bg-white p-3 rounded-xl border border-gray-100 shadow-sm">
              <div class="w-8 h-8 bg-blue-100 rounded-full flex items-center justify-center flex-shrink-0 mt-1">
                <i class="fa-solid fa-shield-check text-blue-600 text-sm"></i>
              </div>
              <p class="text-xs text-gray-600">
                Được phát triển với mục tiêu bảo vệ cộng đồng khỏi rủi ro lừa đảo trực tuyến.
              </p>
            </div>
          </div>
        </div>
      </div>
    </footer>
  </div>

{:else if pageState === 'investigating'}
  <div class="flex min-h-screen flex-col bg-[#f8fafc]">
    <!-- HEADER -->
    <header class="bg-white/80 backdrop-blur-md sticky top-0 z-50 border-b border-gray-100">
      <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
        <div class="flex justify-between items-center h-20">
          <div class="flex items-center flex-shrink-0 cursor-pointer gap-3">
            <img src="/logo.svg" alt="Logo tracuuluadao.vn" class="h-10 w-auto" />
            <span class="font-bold text-xl text-gray-900">tracuuluadao.vn</span>
          </div>

          <nav class="hidden md:flex space-x-8">
            <a class="text-sm font-medium text-gray-700 hover:text-blue-600 nav-link active flex items-center gap-1" href="/">
              Trang chủ <i class="fa-solid fa-chevron-down text-xs"></i>
            </a>
            <button type="button" class="text-sm font-medium text-gray-700 hover:text-blue-600 flex items-center gap-1">
              Hướng dẫn <i class="fa-solid fa-chevron-down text-xs"></i>
            </button>
            <button type="button" class="text-sm font-medium text-gray-700 hover:text-blue-600 flex items-center gap-1">
              Câu hỏi thường gặp <i class="fa-solid fa-chevron-down text-xs"></i>
            </button>
            <button type="button" class="text-sm font-medium text-gray-700 hover:text-blue-600">Blog</button>
          </nav>

          <div class="flex items-center space-x-4">
            <button class="flex items-center gap-2 px-4 py-2 border border-gray-200 rounded-full text-sm font-medium text-gray-700 hover:bg-gray-50 transition">
              <i class="fa-solid fa-shield-halved text-gray-400"></i> Báo cáo của tôi
            </button>
            <button type="button" aria-label="Tài khoản người dùng" class="w-10 h-10 rounded-full border border-gray-200 flex items-center justify-center text-gray-600 hover:bg-gray-50 transition">
              <i class="fa-regular fa-user"></i>
            </button>
          </div>
        </div>
      </div>
    </header>

    <!-- MAIN -->
    <main class="flex-1 w-full max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8 md:py-12 flex flex-col gap-6">
      <!-- Header Section -->
      <div class="grid grid-cols-1 lg:grid-cols-12 gap-6 items-start">
        <!-- Left Column: Live Feed (8 cols) -->
        <div class="lg:col-span-8 flex flex-col gap-6">
          <div class="flex w-full flex-col">
            <div class="mb-[-1px] flex items-end gap-2">
              <button
                type="button"
                onclick={() => {
                  investigationPanelTab = 'process';
                }}
                class="rounded-t-2xl border border-gray-200 border-b-0 px-4 py-2 text-sm font-medium transition-colors {investigationPanelTab === 'process' ? 'bg-white text-blue-600 shadow-sm' : 'bg-slate-100 text-slate-500 hover:bg-slate-200 hover:text-slate-700'}"
              >
                Quá trình điều tra
              </button>
              <button
                type="button"
                onclick={() => {
                  if (reportTabReady) {
                    investigationPanelTab = 'report';
                    if (!detectiveNarrativeVisible.trim()) {
                      reportNarrativePrimed = true;
                    }
                    if (!hasDetectiveNarrative && currentInvestigationId) {
                      startReportStream(currentInvestigationId);
                    }
                  }
                }}
                class="rounded-t-2xl border border-gray-200 border-b-0 px-4 py-2 text-sm font-medium transition-colors {investigationPanelTab === 'report' ? 'bg-white text-blue-600 shadow-sm' : reportTabReady ? 'bg-slate-100 text-slate-500 hover:bg-slate-200 hover:text-slate-700' : 'cursor-not-allowed bg-slate-100 text-slate-300'}"
                disabled={!reportTabReady}
              >
                Báo cáo
              </button>
            </div>

            <div class="w-full bg-white rounded-r-2xl rounded-b-2xl rounded-tl-none p-6 shadow-sm border border-gray-100 lg:h-[calc(100vh-11.5rem)] overflow-hidden">
            {#if investigationPanelTab === 'report'}
              {#if hasDetectiveNarrative}
                <div class="flex h-full items-start gap-4">
                  <div class="flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl border border-blue-200 bg-blue-50">
                    <img src="/logo.svg" alt="Logo tracuuluadao.vn" class="h-8 w-auto" />
                  </div>
                  <div class="min-w-0 flex-1 h-full flex flex-col">
                    <div class="flex flex-wrap items-center gap-3">
                      <p class="font-medium text-slate-900">{completeEvent ? 'Báo cáo điều tra' : `AI đang viết nhận định cho ${currentQuery}`}</p>
                      <span class="inline-flex items-center gap-2 rounded-full px-3 py-1 text-xs font-medium {completeEvent ? 'bg-emerald-50 text-emerald-700' : 'bg-blue-50 text-blue-700 investigation-live-pill'}">
                        <span class="{completeEvent ? 'h-2 w-2 rounded-full bg-emerald-500' : 'investigation-live-dot h-2 w-2 rounded-full bg-blue-500'}"></span>
                        {completeEvent ? 'Đã hoàn tất' : 'Đang cập nhật'}
                      </span>
                      {#if historicalContext}
                        <span class="inline-flex items-center gap-2 rounded-full bg-indigo-50 px-3 py-1 text-xs font-medium text-indigo-700">
                          <span class="h-2 w-2 rounded-full bg-indigo-500"></span>
                          Đã có lịch sử trước đó
                        </span>
                      {/if}
                    </div>
                    <p class="mt-1 text-sm leading-6 text-slate-500">
                      {completeEvent
                        ? `Hồ sơ ${currentQuery} đã được tổng hợp thành báo cáo cuối cùng.`
                        : 'Báo cáo đang được dựng dần khi điều tra viên ghép thêm bằng chứng mới.'}
                    </p>

                    {#if historicalContext}
                      <div class="mt-4 rounded-2xl border border-indigo-100 bg-indigo-50/80 p-4 text-sm text-slate-700">
                        <p class="font-medium text-slate-900">Dữ liệu lịch sử từ hệ thống</p>
                        <div class="mt-2 grid gap-2 sm:grid-cols-2">
                          <p>Đã từng tra cứu: <strong>{historicalContext.investigation_count}</strong> lần</p>
                          <p>Đánh giá gần nhất: <strong>{formatRiskLevelLabel(historicalContext.last_risk_level as RiskLevel)}</strong></p>
                          <p>Báo cáo đã duyệt: <strong>{historicalContext.approved_reports}</strong></p>
                          <p>Đối tượng liên kết: <strong>{historicalContext.linked_subjects_count}</strong></p>
                          <p>Lần đầu ghi nhận: <strong>{formatDateLabel(historicalContext.first_seen)}</strong></p>
                          <p>Gần nhất cập nhật: <strong>{formatDateLabel(historicalContext.last_seen)}</strong></p>
                        </div>
                      </div>
                    {/if}

                    <article bind:this={reportScrollViewport} class="report-markdown primary-scrollbar mt-6 min-h-0 flex-1 overflow-auto pr-2 text-[15px] leading-8 text-slate-700">
                      {@html formattedDetectiveNarrativeHtml}
                    </article>
                  </div>
                </div>
              {:else}
                <div class="flex items-start gap-4">
                  <div class="flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl border border-slate-200 bg-slate-50">
                    <img src="/logo.svg" alt="Logo tracuuluadao.vn" class="h-8 w-auto opacity-70" />
                  </div>
                  <div class="max-w-[88%] rounded-3xl rounded-tl-md bg-slate-100 px-4 py-3 text-sm leading-7 text-slate-700">
                    {#if reportStreamError}
                      Chưa thể mở báo cáo lúc này. {reportStreamError}
                    {:else if completeEvent}
                      Báo cáo đang được chuẩn bị. Nội dung sẽ xuất hiện sau ít giây nữa.
                    {:else}
                      Tôi đang gom các bằng chứng đầu tiên. Khi điều tra hoàn tất, tab này sẽ hiện báo cáo cho bạn.
                    {/if}
                  </div>
                </div>
              {/if}
            {:else}
              <div class="flex h-full items-start gap-4">
                <div class="flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl border border-blue-200 bg-blue-50">
                  <img src="/logo.svg" alt="Logo tracuuluadao.vn" class="h-8 w-auto" />
                </div>
                <div class="min-w-0 flex-1 h-full flex flex-col">
                  <div class="flex flex-wrap items-center gap-3">
                    <p class="font-medium text-slate-900">Quá trình điều tra</p>
                    {#if activeTimelineStep && !completeEvent}
                      <span class="inline-flex items-center gap-2 rounded-full bg-blue-50 px-3 py-1 text-xs font-medium text-blue-700 investigation-live-pill">
                        <span class="investigation-live-dot h-2 w-2 rounded-full bg-blue-500"></span>
                        {activeTimelineStep.label}
                      </span>
                    {:else if completeEvent}
                      <span class="inline-flex items-center gap-2 rounded-full bg-emerald-50 px-3 py-1 text-xs font-medium text-emerald-700">
                        <span class="h-2 w-2 rounded-full bg-emerald-500"></span>
                        Đã hoàn tất
                      </span>
                    {/if}
                  </div>
                  <article bind:this={processScrollViewport} class="process-markdown primary-scrollbar mt-4 min-h-0 flex-1 overflow-auto pr-2 text-[15px] leading-8 text-slate-700 break-words">
                    {@html formattedProcessNarrativeHtml}
                    {#if activeTimelineStep && !completeEvent}
                      <span class="investigation-search-indicator ml-2 inline-flex h-6 w-6 items-center justify-center align-middle text-blue-500">
                        <i class="fa-solid fa-magnifying-glass text-sm"></i>
                      </span>
                    {/if}
                  </article>
                </div>
              </div>
            {/if}
          </div>
          </div>

          {#if !hasDetectiveNarrative}
            <div class="bg-amber-50 border border-amber-200 rounded-xl p-5 flex gap-4 items-start">
              <div class="text-amber-600 text-2xl">
                <i class="fa-solid fa-lightbulb"></i>
              </div>
              <div>
                <h4 class="font-medium text-gray-900 mb-1">Bạn biết không?</h4>
                <p class="text-sm text-gray-500">Chúng tôi sử dụng hàng trăm công cụ và AI agents để truy quét, phân tích và xác minh thông tin từ hàng ngàn nguồn trên internet.</p>
              </div>
            </div>
          {/if}
        </div>

        <!-- Right Column: Results Summary (4 cols) -->
        <div class="lg:col-span-4 flex flex-col gap-6">
          <!-- Risk Meter Card -->
          <div class="bg-white rounded-2xl p-6 shadow-sm border border-gray-100 shrink-0">
            <div class="flex justify-between items-center mb-4">
              <h2 class="text-xl font-medium text-gray-900">Kết luận sơ bộ</h2>
              <span class="text-xs bg-gray-200 text-gray-500 px-2 py-1 rounded">{completeEvent ? 'Đã chốt' : 'Cập nhật liên tục'}</span>
            </div>

            <div class="{riskCard.panel} border rounded-xl p-6 text-center mb-6">
              <div class="w-16 h-16 mx-auto rounded-full flex items-center justify-center mb-3 {riskCard.iconWrap}">
                <i class="{riskCard.icon} text-4xl"></i>
              </div>
              <h3 class="text-xl font-bold text-gray-900 mb-2">{riskCard.title}</h3>
              <p class="text-sm text-gray-500">{riskCard.description}</p>
            </div>

            <div>
              <div class="flex justify-between text-sm mb-2">
                <span class="font-medium text-gray-900">{completeEvent ? 'Độ chắc chắn' : 'Tiến độ điều tra'}</span>
                <span class="font-bold">{completeEvent ? `${conclusionMeterPercent}%` : `${investigationProgress}%`}</span>
              </div>
              <div class="w-full bg-gray-200 rounded-full h-2.5 mb-2">
                <div class="{riskCard.bar} h-2.5 rounded-full" style={`width: ${completeEvent ? conclusionMeterPercent : investigationProgress}%`}></div>
              </div>
              <p class="text-xs text-gray-500">
                {#if completeEvent}
                  {confidenceDescription(completeEvent.confidence)} Dữ liệu được tổng hợp từ {completeEvent.sources_analyzed} nguồn trong {formatDuration(completeEvent.duration_ms)}.
                {:else if sourceStats.total > 0}
                  Đã nhận dữ liệu từ {sourceStats.total} nguồn và đang tiếp tục hợp nhất bằng chứng.
                {:else}
                  Chưa có nguồn nào phản hồi, hệ thống vẫn đang mở rộng truy quét.
                {/if}
              </p>
            </div>
          </div>

          <!-- Summary Card -->
          <div class="bg-white rounded-2xl p-6 shadow-sm border border-gray-100 min-h-0 flex flex-col">
            <h2 class="text-xl font-medium text-gray-900 mb-4">Tóm tắt phát hiện</h2>
            <ul class="primary-scrollbar space-y-4 mb-6 min-h-0 flex-1 overflow-auto pr-1">
              {#each summaryItems as item}
                <li class="flex gap-3 text-sm">
                  <span class="w-2 h-2 rounded-full mt-1.5 flex-shrink-0 {item.color}"></span>
                  <span>{item.text}</span>
                </li>
              {/each}
            </ul>
            <button
              type="button"
              onclick={() => {
                closeInvestigation();
                pageState = 'home';
              }}
              class="w-full bg-blue-50 hover:bg-blue-100 text-blue-600 font-medium py-2 px-4 rounded-lg flex items-center justify-center gap-2 transition-colors"
            >
              Tra cứu một mục khác
              <i class="fa-solid fa-arrow-right text-sm"></i>
            </button>
          </div>

        </div>
      </div>

      {#if completeEvent}
        <div class="grid grid-cols-1 {linkedSubjects.length > 0 ? 'md:grid-cols-2' : ''} gap-6">
          {#if linkedSubjects.length > 0}
            <div class="bg-white rounded-2xl p-6 shadow-sm border border-gray-100">
              <div class="flex items-center justify-between gap-3">
                <h2 class="text-xl font-medium text-gray-900">Đối tượng liên kết</h2>
              </div>
              <div class="mt-4 space-y-3">
                {#each linkedSubjects.slice(0, 8) as subject (subject.id)}
                  <div class="rounded-xl border border-slate-200 bg-slate-50 px-4 py-3">
                    <div class="flex items-start justify-between gap-3">
                      <div>
                        <p class="font-medium text-slate-900 break-all">{subject.value}</p>
                        <p class="mt-1 text-xs uppercase tracking-[0.16em] text-slate-500">{subject.subject_type}</p>
                      </div>
                      <span class="rounded-full bg-white px-2.5 py-1 text-xs font-medium text-slate-600">
                        cấp {subject.depth}
                      </span>
                    </div>
                    <div class="mt-2 flex flex-wrap gap-2 text-xs text-slate-600">
                      <span class="rounded-full bg-white px-2.5 py-1">Rủi ro: {formatRiskLevelLabel(subject.risk_level as RiskLevel)}</span>
                      <span class="rounded-full bg-white px-2.5 py-1">Điểm {Math.round(subject.risk_score * 100)}%</span>
                    </div>
                  </div>
                {/each}
              </div>
            </div>
          {/if}

          <div class="bg-white rounded-2xl p-6 shadow-sm border border-gray-100">
            <h2 class="text-xl font-medium text-gray-900">Báo cáo từ cộng đồng</h2>
            <p class="mt-2 text-sm leading-6 text-slate-500">
              Nếu bạn từng gặp trường hợp này, hãy gửi thêm mô tả để đội ngũ duyệt và bổ sung vào kho dữ liệu.
            </p>
            <div class="mt-4 space-y-4">
              <label class="block text-sm">
                <span class="mb-2 block font-medium text-slate-800">Loại báo cáo</span>
                <select bind:value={reportCategory} class="w-full rounded-xl border border-slate-200 bg-white px-3 py-2.5 text-sm text-slate-800 outline-none transition focus:border-blue-400">
                  {#each reportCategoryOptions as option}
                    <option value={option.value}>{option.label}</option>
                  {/each}
                </select>
              </label>
              <label class="block text-sm">
                <span class="mb-2 block font-medium text-slate-800">Mô tả thêm</span>
                <textarea
                  bind:value={reportDescription}
                  rows="5"
                  maxlength="2000"
                  placeholder="Ví dụ: số này gọi mạo danh ngân hàng, yêu cầu cài app hoặc chuyển tiền..."
                  class="w-full rounded-2xl border border-slate-200 bg-white px-3 py-3 text-sm leading-6 text-slate-800 outline-none transition focus:border-blue-400"
                ></textarea>
              </label>
              <div class="flex items-center justify-between gap-4 text-xs text-slate-500">
                <span>Tối đa 2000 ký tự.</span>
                <span>{reportDescription.length}/2000</span>
              </div>
              {#if reportSubmitError}
                <p class="text-sm text-red-600">{reportSubmitError}</p>
              {/if}
              {#if reportSubmitMessage}
                <p class="text-sm text-emerald-600">{reportSubmitMessage}</p>
              {/if}
              <button
                type="button"
                onclick={submitCommunityReport}
                disabled={reportSubmitting}
                class="w-full rounded-xl bg-slate-900 px-4 py-3 text-sm font-medium text-white transition hover:bg-slate-800 disabled:cursor-not-allowed disabled:bg-slate-400"
              >
                {reportSubmitting ? 'Đang gửi báo cáo...' : 'Gửi báo cáo cộng đồng'}
              </button>
            </div>
          </div>
        </div>
      {/if}
    </main>

    <!-- FOOTER -->
    <footer class="bg-gray-50 border-t border-gray-200 py-16 mt-auto">
      <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
        <div class="grid grid-cols-1 md:grid-cols-4 lg:grid-cols-5 gap-12">
          <div class="lg:col-span-2">
            <div class="flex items-center flex-shrink-0 mb-4 gap-3">
              <img src="/logo.svg" alt="Logo tracuuluadao.vn" class="h-10 w-auto" />
              <span class="font-bold text-xl text-gray-900">tracuuluadao.vn</span>
            </div>
            <p class="text-sm text-gray-600 max-w-xs mb-6">
              Nền tảng tra cứu và phân tích thông tin đáng tin cậy, bảo vệ bạn khỏi rủi ro lừa đảo.
            </p>
          </div>

          <div>
            <h4 class="font-bold text-gray-900 mb-4">Về chúng tôi</h4>
            <ul class="space-y-3">
              <li><button type="button" class="text-sm text-gray-600 hover:text-blue-600">Giới thiệu</button></li>
              <li><button type="button" class="text-sm text-gray-600 hover:text-blue-600">Điều khoản sử dụng</button></li>
              <li><button type="button" class="text-sm text-gray-600 hover:text-blue-600">Chính sách bảo mật</button></li>
            </ul>
          </div>

          <div>
            <h4 class="font-bold text-gray-900 mb-4">Hỗ trợ</h4>
            <ul class="space-y-3">
              <li><button type="button" class="text-sm text-gray-600 hover:text-blue-600">Hướng dẫn sử dụng</button></li>
              <li><button type="button" class="text-sm text-gray-600 hover:text-blue-600">Câu hỏi thường gặp</button></li>
              <li><button type="button" class="text-sm text-gray-600 hover:text-blue-600">Liên hệ hỗ trợ</button></li>
            </ul>
          </div>

          <div>
            <h4 class="font-bold text-gray-900 mb-4">Kết nối với chúng tôi</h4>
            <div class="flex space-x-4 mb-6">
              <button type="button" aria-label="Facebook" class="w-10 h-10 rounded-full bg-gray-200 flex items-center justify-center text-gray-600 hover:bg-blue-600 hover:text-white transition">
                <i class="fa-brands fa-facebook-f"></i>
              </button>
              <button type="button" aria-label="Twitter" class="w-10 h-10 rounded-full bg-gray-200 flex items-center justify-center text-gray-600 hover:bg-blue-400 hover:text-white transition">
                <i class="fa-brands fa-twitter"></i>
              </button>
              <button type="button" aria-label="Email" class="w-10 h-10 rounded-full bg-gray-200 flex items-center justify-center text-gray-600 hover:bg-red-500 hover:text-white transition">
                <i class="fa-regular fa-envelope"></i>
              </button>
            </div>
            <div class="flex items-start gap-3 bg-white p-3 rounded-xl border border-gray-100 shadow-sm">
              <div class="w-8 h-8 bg-blue-100 rounded-full flex items-center justify-center flex-shrink-0 mt-1">
                <i class="fa-solid fa-shield-check text-blue-600 text-sm"></i>
              </div>
              <p class="text-xs text-gray-600">
                Được phát triển với mục tiêu bảo vệ cộng đồng khỏi rủi ro lừa đảo trực tuyến.
              </p>
            </div>
          </div>
        </div>
      </div>
    </footer>
  </div>
{/if}

<style>
  .investigation-live-dot {
    animation: investigation-pulse 1.4s ease-in-out infinite;
  }

  .investigation-live-pill {
    animation: investigation-soft-glow 1.8s ease-in-out infinite;
  }

  .investigation-live-line {
    animation: investigation-soft-glow 1.8s ease-in-out infinite;
  }

  .investigation-search-indicator {
    animation: investigation-search-sway 1.2s ease-in-out infinite;
    transform-origin: 65% 65%;
  }

  @keyframes investigation-pulse {
    0%, 100% { opacity: 1; transform: scale(1); }
    50% { opacity: 0.35; transform: scale(0.88); }
  }

  @keyframes investigation-soft-glow {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.62; }
  }

  @keyframes investigation-search-sway {
    0%, 100% { opacity: 0.9; transform: rotate(-12deg) translateY(0); }
    50% { opacity: 0.45; transform: rotate(10deg) translateY(-1px); }
  }

  .primary-scrollbar {
    scrollbar-width: thin;
    scrollbar-color: #2563eb #dbeafe;
  }

  .primary-scrollbar::-webkit-scrollbar {
    width: 10px;
    height: 10px;
  }

  .primary-scrollbar::-webkit-scrollbar-track {
    background: #dbeafe;
    border-radius: 9999px;
  }

  .primary-scrollbar::-webkit-scrollbar-thumb {
    background: linear-gradient(180deg, #3b82f6 0%, #2563eb 100%);
    border-radius: 9999px;
    border: 2px solid #dbeafe;
  }

  .primary-scrollbar::-webkit-scrollbar-thumb:hover {
    background: linear-gradient(180deg, #2563eb 0%, #1d4ed8 100%);
  }

  .report-markdown :global(h1),
  .process-markdown :global(h1),
  .report-markdown :global(h2),
  .process-markdown :global(h2),
  .report-markdown :global(h3) {
    color: #0f172a;
    font-weight: 700;
    letter-spacing: -0.01em;
  }

  .process-markdown :global(h3) {
    color: #0f172a;
    font-weight: 700;
    letter-spacing: -0.01em;
  }

  .report-markdown :global(h2),
  .process-markdown :global(h2) {
    margin-top: 2rem;
    margin-bottom: 0.85rem;
    font-size: 1.15rem;
  }

  .report-markdown :global(h2) {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    margin-top: 2.2rem;
    padding-top: 1rem;
    border-top: 1px solid #dbeafe;
    color: #0f172a;
  }

  .report-markdown :global(h2)::before {
    content: "";
    width: 0.6rem;
    height: 0.6rem;
    border-radius: 9999px;
    background: linear-gradient(180deg, #2563eb 0%, #1d4ed8 100%);
    box-shadow: 0 0 0 6px rgba(37, 99, 235, 0.09);
    flex-shrink: 0;
  }

  .report-markdown :global(p),
  .process-markdown :global(p) {
    margin: 0 0 1rem;
    color: #334155;
  }

  .report-markdown :global(p:first-of-type) {
    margin-bottom: 1.25rem;
    color: #1e293b;
    font-size: 1rem;
    line-height: 1.9;
  }

  .report-markdown :global(strong),
  .process-markdown :global(strong) {
    color: #0f172a;
    font-weight: 700;
  }

  .report-markdown :global(strong) {
    color: #172554;
    font-weight: 800;
  }

  .report-markdown :global(ul),
  .process-markdown :global(ul) {
    margin: 1rem 0;
    padding-left: 1.2rem;
    list-style: disc;
  }

  .report-markdown :global(li),
  .process-markdown :global(li) {
    margin-bottom: 0.7rem;
    color: #334155;
  }

  .report-markdown :global(ul) {
    padding-left: 0;
    list-style: none;
  }

  .report-markdown :global(ul li) {
    position: relative;
    margin-bottom: 0.85rem;
    padding: 0.95rem 1rem 0.95rem 2.9rem;
    border: 1px solid #dbeafe;
    border-radius: 1rem;
    background: linear-gradient(180deg, #f8fbff 0%, #eef6ff 100%);
    color: #1e293b;
  }

  .report-markdown :global(ul li)::before {
    content: "!";
    position: absolute;
    left: 1rem;
    top: 0.92rem;
    display: flex;
    width: 1.35rem;
    height: 1.35rem;
    align-items: center;
    justify-content: center;
    border-radius: 9999px;
    background: #dbeafe;
    color: #1d4ed8;
    font-size: 0.78rem;
    font-weight: 800;
  }

  .process-markdown :global(ol) {
    margin: 1rem 0;
    padding-left: 1.3rem;
    list-style: decimal;
  }

  .process-markdown :global(blockquote) {
    margin: 1.1rem 0;
    padding: 0.9rem 1rem;
    border-left: 3px solid #2563eb;
    background: #eff6ff;
    color: #1e3a8a;
    border-radius: 0 1rem 1rem 0;
  }

  .process-markdown :global(a) {
    color: #2563eb;
    font-weight: 500;
    text-decoration: none;
    word-break: break-all;
  }

  .process-markdown :global(a:hover) {
    color: #1d4ed8;
    text-decoration: underline;
  }

  .report-markdown :global(hr),
  .process-markdown :global(hr) {
    margin: 1.5rem 0;
    border: 0;
    border-top: 1px solid #e2e8f0;
  }

  .report-markdown :global(.report-critical) {
    display: inline-block;
    padding: 0.12rem 0.45rem;
    border-radius: 9999px;
    background: #fef2f2;
    color: #b91c1c;
    font-weight: 800;
    box-shadow: inset 0 0 0 1px rgba(248, 113, 113, 0.24);
  }

  .report-markdown :global(.report-emphasis) {
    color: #0f172a;
    font-weight: 700;
  }

  .progress-line {
    width: 2px;
    background-color: #e5e7eb;
    position: absolute;
    left: 20px;
    top: 40px;
    bottom: -10px;
    z-index: 0;
  }

  .progress-line-active {
    background-color: #2563eb;
  }

  /* ── Navbar active link ── */
  .nav-link.active { color: #2563eb; font-weight: 700; }

  /* ── Gradient text ── */
  .gradient-text {
    background: linear-gradient(135deg, #2563eb 0%, #0ea5e9 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
  }

  /* ── Gradient button ── */
  .gradient-btn {
    background: linear-gradient(135deg, #1d4ed8 0%, #0ea5e9 100%);
    transition: all 0.2s ease;
    box-shadow: 0 4px 20px rgba(14,165,233,0.35);
  }
  .gradient-btn:hover {
    transform: translateY(-1px);
    box-shadow: 0 8px 28px rgba(14,165,233,0.5);
  }
  .gradient-btn:active { transform: translateY(0); }

</style>
