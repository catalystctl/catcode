import { Chat } from "@/components/chat";
import { ErrorBoundary } from "@/components/error-boundary";

export default function Page() {
  return (
    <ErrorBoundary label="app">
      <Chat />
    </ErrorBoundary>
  );
}
