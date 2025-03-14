import {
  FaBan,
  FaCheckCircle,
  FaExternalLinkAlt,
  FaSpinner,
  FaTimesCircle,
} from "react-icons/fa";
import { PipelineCondition, PipelineConditionType } from "../types/pipeline";

interface PipelineStepContainerProps {
  index: number;
  children: React.ReactNode;
  conditions: PipelineCondition[];
  status?: "Pending" | "Completed" | "Failed" | "Cancelled";
  transactionHash: string | null;
  error: string | null;
}

export const PipelineStepContainer = ({
  index,
  children,
  conditions,
  status,
  transactionHash,
  error,
}: PipelineStepContainerProps) => {
  return (
    <div className="border border-purple-500/30 rounded-lg lg:p-4 p-4 bg-black/40 backdrop-blur-sm">
      <div className="flex items-center gap-4">
        <div className="text-sm text-purple-300 lg:inline hidden">
          {index + 1}
        </div>
        {children}
      </div>

      {/* Conditions */}
      {conditions.length > 0 && (
        <div className="mt-3 pt-3 border-t border-purple-500/30">
          <div className="text-sm text-purple-300">Conditions:</div>
          {conditions.map((condition, index) => (
            <div
              key={index}
              className="mt-1 lg:text-sm text-xs text-purple-200"
            >
              {condition.type === PipelineConditionType.Now
                ? "Execute immediately"
                : condition.type === PipelineConditionType.PriceAbove
                  ? `Price above ${condition.value} for ${condition.asset.slice(0, 4)}...${condition.asset.slice(-4)}`
                  : `Price below ${condition.value} for ${condition.asset.slice(0, 4)}...${condition.asset.slice(-4)}`}
            </div>
          ))}
        </div>
      )}
      {status && (
        <div className="mt-3 pt-3 border-t border-purple-500/30">
          <div className="text-sm text-purple-300">Status:</div>
          <TransactionLink
            status={status}
            transactionHash={transactionHash}
            error={error}
          />
        </div>
      )}
    </div>
  );
};

const renderStatus = (status: string) => {
  switch (status) {
    case "Pending":
      return (
        <span className="text-yellow-300 flex items-center gap-1">
          <FaSpinner /> Pending
        </span>
      );
    case "Completed":
      return (
        <span className="text-green-300 flex items-center gap-1">
          <FaCheckCircle /> Completed
        </span>
      );
    case "Failed":
      return (
        <span className="text-red-300 flex items-center gap-1">
          <FaTimesCircle /> Failed:
        </span>
      );
    case "Cancelled":
      return (
        <span className="text-gray-300 flex items-center gap-1">
          <FaBan /> Cancelled
        </span>
      );
  }
};

function formatError(error: string) {
  if (error.includes("insufficient funds")) {
    return "Insufficient balance";
  }
  if (error.includes("0x1771")) {
    return "Slippage tolerance exceeded";
  }
  try {
    // Look for JSON between curly braces
    const match = error.match(/{.*}/);
    if (match) {
      const parsedError = JSON.parse(match[0]);
      if (parsedError?.error) {
        return JSON.stringify(parsedError.error);
      }
    }
    return error;
  } catch {
    return error;
  }
}

export const TransactionLink = ({
  status,
  transactionHash,
  error,
}: {
  status: string;
  transactionHash: string | null;
  error: string | null;
}) => {
  return (
    <div className="text-xs sm:text-sm text-gray-400 flex flex-wrap items-center gap-2 mt-2">
      {renderStatus(status)}{" "}
      {transactionHash && (
        <span className="flex items-center gap-1 inline-flex">
          <a
            href={
              transactionHash.startsWith("0x")
                ? `https://blockscan.com/tx/${transactionHash}`
                : `https://solscan.io/tx/${transactionHash}`
            }
            target="_blank"
            rel="noopener noreferrer"
            className="text-blue-400 hover:text-blue-300 inline-flex items-center gap-1"
          >
            {transactionHash.slice(0, 6)}...{transactionHash.slice(-4)}
            <FaExternalLinkAlt size={10} />
          </a>
        </span>
      )}
      {error && (
        <span className="text-red-300 break-all overflow-hidden">
          {formatError(error)}
        </span>
      )}
    </div>
  );
};
