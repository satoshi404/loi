;;; emacs-loi.el --- Loi Flow Syntax and LSP support -*- lexical-binding: t; -*-
;;
;; Usage:
;;   (setq loi-lsp-command '("/absolute/path/to/loi-lsp"))
;;   (load "/absolute/path/to/loi/editors/emacs-loi.el")
;;
;; Build the server first with `sh tools/build_lsp_server.sh`.
;; Eglot is built into Emacs 29+; `M-x eglot` starts the session in a .loi buffer.
;; If you use lsp-mode instead, require it before loading this file.

(defgroup loi nil
  "Loi language support."
  :group 'languages)

(defcustom loi-lsp-command '("loi-lsp")
  "Command used to start the Loi language server.
Use an absolute path, or add the binary to `exec-path'."
  :type '(repeat string)
  :group 'loi)

(define-derived-mode loi-mode prog-mode "Loi"
  "Major mode for Loi Flow Syntax."
  (setq-local comment-start "// ")
  (setq-local comment-end ""))

(add-to-list 'auto-mode-alist '("\\.loi\\'" . loi-mode))

(defun loi--eglot-contact (_interactive)
  "Return the eglot contact for the Loi language server."
  loi-lsp-command)

(with-eval-after-load 'eglot
  (add-to-list 'eglot-server-programs '(loi-mode . loi--eglot-contact)))

(with-eval-after-load 'lsp-mode
  (lsp-register-client
   (make-lsp-client
    :new-connection (lsp-stdio-connection (lambda () loi-lsp-command))
    :major-modes '(loi-mode)
    :server-id 'loi-lsp)))

(provide 'emacs-loi)
;;; emacs-loi.el ends here
