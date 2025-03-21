import {
  useCreateWallet,
  useDelegatedActions,
  usePrivy,
  useSolanaWallets,
  useWallets,
  type WalletWithMetadata,
} from "@privy-io/react-auth";
import { useState } from "react";

export function DelegateActionButton() {
  const { user } = usePrivy();
  const {
    ready: solanaReady,
    wallets: solanaWallets,
    createWallet: createSolanaWallet,
  } = useSolanaWallets();
  const { ready: evmReady, wallets: evmWallets } = useWallets();
  const { createWallet: createEvmWallet } = useCreateWallet();
  const { delegateWallet } = useDelegatedActions();

  const [isCreatingSolana, setIsCreatingSolana] = useState(false);
  const [isCreatingEvm, setIsCreatingEvm] = useState(false);

  const onCreateWallet = async () => {
    try {
      setIsCreatingSolana(true);
      await createSolanaWallet();
    } catch (error) {
      console.error("Error creating Solana wallet:", error);
    } finally {
      setIsCreatingSolana(false);
    }

    try {
      setIsCreatingEvm(true);
      await createEvmWallet();
    } catch (error) {
      console.error("Error creating EVM wallet:", error);
    } finally {
      setIsCreatingEvm(false);
    }
  };

  // Find both Solana and EVM embedded wallets to delegate
  const solanaWalletToDelegate = solanaWallets.find(
    (wallet) => wallet.walletClientType === "privy"
  );

  const evmWalletToDelegate = evmWallets.find(
    (wallet) => wallet.walletClientType === "privy"
  );

  // Check delegation status for each chain type
  const isSolanaDelegated = user?.linkedAccounts.some(
    (account): account is WalletWithMetadata =>
      account.type === "wallet" &&
      account.delegated &&
      account.chainType === "solana"
  );

  const isEvmDelegated = user?.linkedAccounts.some(
    (account): account is WalletWithMetadata =>
      account.type === "wallet" &&
      account.delegated &&
      account.chainType === "ethereum"
  );

  if (
    (solanaReady && !solanaWalletToDelegate) ||
    (evmReady && !evmWalletToDelegate)
  ) {
    return (
      <button
        disabled={
          !solanaReady || !evmReady || isCreatingSolana || isCreatingEvm
        }
        onClick={onCreateWallet}
        className="p-2 border-2 border-purple-500/30 rounded-lg bg-black/40 backdrop-blur-sm flex items-center px-3 text-sm hover:bg-purple-500/10"
      >
        {isCreatingSolana || isCreatingEvm ? (
          <span>Creating wallet...</span>
        ) : (
          <span>
            Create {solanaReady && !solanaWalletToDelegate ? "Solana" : ""}{" "}
            {evmReady && !evmWalletToDelegate ? "EVM" : ""} wallet
          </span>
        )}
      </button>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      {!isSolanaDelegated && (
        <button
          disabled={!solanaReady || !solanaWalletToDelegate}
          onClick={async () => {
            try {
              await delegateWallet({
                address: solanaWalletToDelegate!.address,
                chainType: "solana",
              });
            } catch (error) {
              console.error("Error delegating Solana wallet:", error);
            }
          }}
          className="p-2 border-2 border-purple-500/30 rounded-lg bg-black/40 backdrop-blur-sm flex items-center px-3 text-sm hover:bg-purple-500/10"
        >
          Delegate Solana
        </button>
      )}
      {isSolanaDelegated && !isEvmDelegated && (
        <button
          disabled={!evmReady || !evmWalletToDelegate}
          onClick={async () => {
            try {
              await delegateWallet({
                address: evmWalletToDelegate!.address,
                chainType: "ethereum",
              });
            } catch (error) {
              console.error("Error delegating EVM wallet:", error);
            }
          }}
          className="p-2 border-2 border-purple-500/30 rounded-lg bg-black/40 backdrop-blur-sm flex items-center px-3 text-sm hover:bg-purple-500/10"
        >
          Delegate EVM
        </button>
      )}
    </div>
  );
}
