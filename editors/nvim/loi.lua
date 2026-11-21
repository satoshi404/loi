-- Loi Flow Syntax and LSP support for Neovim.
--
-- Usage (from init.lua):
--   vim.g.loi_lsp_cmd = "/absolute/path/to/loi-lsp"
--   dofile("/absolute/path/to/loi/editors/nvim/loi.lua")
--
-- `cargo build` produces the separate `loi-lsp` executable next to `loi`.
-- Neovim 0.11+ uses the built-in vim.lsp.config API; older releases fall back
-- to nvim-lspconfig when it is installed.

vim.filetype.add({ extension = { loi = "loi" } })

vim.api.nvim_create_autocmd("FileType", {
  pattern = "loi",
  callback = function()
    vim.bo.commentstring = "// %s"
  end,
})

local cmd = { vim.g.loi_lsp_cmd or "loi-lsp" }

if vim.lsp.config then
  vim.lsp.config("loi_lsp", {
    cmd = cmd,
    filetypes = { "loi" },
    root_markers = { ".git" },
  })
  vim.lsp.enable("loi_lsp")
else
  local ok, configs = pcall(require, "lspconfig.configs")
  if ok then
    local util = require("lspconfig.util")
    configs.loi_lsp = {
      default_config = {
        cmd = cmd,
        filetypes = { "loi" },
        root_dir = function(fname)
          return util.find_git_ancestor(fname) or vim.fn.getcwd()
        end,
        single_file_support = true,
      },
    }
    require("lspconfig").loi_lsp.setup({})
  end
end
