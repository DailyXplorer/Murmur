import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type MeetingMarkdownKind } from "@/bindings";

/**
 * Copy and Export for a tab: the backend writes the Markdown for both, so
 * the copied and the saved text are the same.
 */
export const useMeetingMarkdown = (id: number, kind: MeetingMarkdownKind) => {
  const { t } = useTranslation();

  const copy = async () => {
    try {
      const result = await commands.copyMeetingMarkdown(id, kind);
      if (result.status === "error") throw new Error(result.error);
      toast.success(t(`meetings.${kind}.copied`));
    } catch (error) {
      console.error(`Failed to copy the ${kind}:`, error);
      toast.error(t(`meetings.${kind}.copyError`));
    }
  };

  const exportMarkdown = async () => {
    try {
      const result = await commands.exportMeetingMarkdown(id, kind);
      if (result.status === "error") throw new Error(result.error);
      if (result.data) toast.success(t(`meetings.${kind}.exported`));
    } catch (error) {
      console.error(`Failed to export the ${kind}:`, error);
      toast.error(t(`meetings.${kind}.exportError`));
    }
  };

  return { copy, exportMarkdown };
};
