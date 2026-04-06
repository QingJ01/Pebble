import { useMutation, useQueryClient } from "@tanstack/react-query";
import { sendEmail } from "@/lib/api";

interface SendEmailParams {
  accountId: string;
  to: string[];
  cc: string[];
  bcc: string[];
  subject: string;
  bodyText: string;
  bodyHtml?: string;
  inReplyTo?: string;
  attachmentPaths?: string[];
}

export function useSendEmailMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (params: SendEmailParams) =>
      sendEmail(
        params.accountId,
        params.to,
        params.cc,
        params.bcc,
        params.subject,
        params.bodyText,
        params.bodyHtml,
        params.inReplyTo,
        params.attachmentPaths,
      ),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["messages"] });
    },
  });
}
