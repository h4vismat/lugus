export type CompanySummary = {
  id: string;
  name: string;
  hint: string;
  conversation_id: string;
  revision: number;
  updated_at: string;
};
export type Finding = {
  message_id: string;
  run_id: string;
  text: string;
  evidence: { kind: string; id: string }[];
  created_at: string;
};
export type Company = CompanySummary & {
  thesis: string;
  questions: string;
  findings: Finding[];
};
export type CompanyHistory = {
  revisions: {
    revision: number;
    thesis: string;
    questions: string;
    updated_at: string;
  }[];
  reviews: {
    run_id: string | null;
    created_at: string;
    brief: string;
    status: string;
    text: string | null;
    error?: { message: string } | null;
  }[];
  next_offset: number | null;
};
