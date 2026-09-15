;;; nostrdown-mode.el --- Nostr {{ }} reference layer over Org, Markdown and AsciiDoc  -*- lexical-binding: t; -*-

;; Author: limina1
;; Version: 0.1.0
;; Package-Requires: ((emacs "28.1"))
;; Keywords: hypermedia, wp, nostr
;; URL: https://github.com/limina1/tendrl-engine

;;; Commentary:

;; A minor mode that layers the nostrdown reference syntax (see
;; docs/nostrdown-spec.md in tendrl-engine) on top of whatever markup
;; the buffer is written in.  It does not replace `org-mode',
;; `markdown-mode' or `adoc-mode'; it sits beside them:
;;
;;   {{ref:The Ascent}}  {{wiki:proof of work|PoW}}  {{embed:naddr1…}}
;;   {{quote:naddr1… | excerpt}}  {{slot:naddr1…}}  {{@npub1…}}  [[topic]]
;;
;; Font-lock recognises the `{{ }}' tokens instantly (a display-only
;; regexp).  Everything algorithmic -- the grammar proper, NIP-54 slug
;; normalisation, sibling lookup, kind-30818 wiki lookup, entity
;; decoding, transclusion -- is asked of a running tendrl engine through
;; `POST /api/v1/nostrdown/resolve', and the answer is painted onto the
;; buffer as overlays: resolved / unresolved / pending faces, a hover
;; description, RET or mouse-1 to follow, optional pretty labels and
;; optional inline transclusion cards for embeds.
;;
;; When no engine is reachable the mode degrades to a `nak' backend:
;; entity targets (naddr / nevent / note / npub / nprofile) are fetched
;; with `nak fetch', `{{ref:}}' targets resolve against files in the
;; same directory, and wiki topics stay unresolved.
;;
;; Sibling context for `{{ref:}}': by default the other files in the
;; buffer's directory act as the sibling sections (file basename = d-tag,
;; title from `#+TITLE:' / `# Heading' / `= Heading'), which is exactly
;; the shape of a kasten directory.  Point the buffer at a published
;; publication instead with a file-local `nostrdown-publication'
;; ("30040:<pubkey>:<d-tag>" or an naddr) or a `#+NOSTRDOWN_PUBLICATION:'
;; keyword.
;;
;; Setup:
;;
;;   (require 'nostrdown-mode)
;;   M-x nostrdown                   ; overlay this buffer now (C-u: off)
;;   (global-nostrdown-mode 1)       ; or: always on in org, markdown, adoc buffers
;;
;; Keys (prefix `nostrdown-keymap-prefix', default C-c n):
;;   r  resolve now (local)      R  resolve online (relay backfill)
;;   f  follow at point          F  fetch pending embed at point
;;   n / p  next / previous reference
;;   i  insert a reference       e  toggle inline embed cards
;;   l  cycle prettify (off / hide markup / show label)
;;   d  describe reference at point

;;; Code:

(require 'json)
(require 'url)
(require 'url-http)
(require 'subr-x)
(require 'seq)

(defvar url-http-end-of-headers)
(defvar nostrdown-mode)

;;;; Customisation

(defgroup nostrdown nil
  "Nostr {{ }} reference layer over prose markups."
  :group 'text
  :prefix "nostrdown-")

(defcustom nostrdown-engine-url "http://localhost:3030"
  "Base URL of a running tendrl engine."
  :type 'string)

(defcustom nostrdown-backend 'auto
  "Which resolver to use.
`auto' uses the engine when it answers `/health' and falls back to
`nak'; `tendrl' and `nak' force one backend."
  :type '(choice (const auto) (const tendrl) (const nak)))

(defcustom nostrdown-nak-program "nak"
  "The nak executable used by the fallback backend."
  :type 'string)

(defcustom nostrdown-request-timeout 8
  "Seconds to wait for a synchronous engine request."
  :type 'integer)

(defcustom nostrdown-idle-delay 0.7
  "Idle seconds after an edit before the buffer is re-resolved."
  :type 'number)

(defcustom nostrdown-idle-policy "local_only"
  "Fetch policy for the automatic (idle) resolve.
`local_only' never touches the network and never blocks on the
engine's Confirm mode; use `nostrdown-resolve-online' for a relay pass."
  :type '(choice (const "local_only") (const "local_first") (const "fetch_always")))

(defcustom nostrdown-online-policy "local_first"
  "Fetch policy for `nostrdown-resolve-online'."
  :type '(choice (const "local_first") (const "fetch_always")))

(defcustom nostrdown-prettify 'markup
  "How a resolved token is displayed.
nil shows the raw token; `markup' hides the `{{prefix:' and `}}' chrome
(and the target when a |display is given), like `org-link-descriptive';
`label' replaces the whole token with the engine's resolved label.
Point entering a token always reveals the raw text."
  :type '(choice (const nil) (const markup) (const label)))

(defcustom nostrdown-inline-embeds nil
  "When non-nil, render `embed' / `quote' / `slot' targets as a card
below the token, using the transcluded content the engine returned."
  :type 'boolean)

(defcustom nostrdown-embed-max-lines 10
  "Maximum content lines shown in an inline embed card."
  :type 'integer)

(defcustom nostrdown-modes '(org-mode markdown-mode gfm-mode adoc-mode)
  "Major modes in which `global-nostrdown-mode' turns the mode on."
  :type '(repeat symbol))

(defcustom nostrdown-sibling-extensions '("org" "md" "markdown" "adoc" "asciidoc" "txt")
  "File extensions that count as sibling sections in a directory."
  :type '(repeat string))

(defcustom nostrdown-keymap-prefix "C-c n"
  "Prefix key for `nostrdown-command-map'."
  :type 'string)

;;;; Faces

(defface nostrdown-delimiter-face '((t :inherit shadow))
  "Face for the `{{' / `}}' / `|' chrome of a token.")
(defface nostrdown-prefix-face '((t :inherit font-lock-keyword-face))
  "Face for the `ref:' / `wiki:' / `@' prefix of a token.")
(defface nostrdown-target-face '((t :inherit font-lock-constant-face))
  "Face for a token's target.")
(defface nostrdown-display-face '((t :inherit font-lock-string-face))
  "Face for a token's `|display' text.")
(defface nostrdown-resolved-face '((t :inherit link))
  "Face for a reference the engine resolved.")
(defface nostrdown-unresolved-face '((t :inherit warning :underline (:style wave)))
  "Face for a reference nothing matched.")
(defface nostrdown-pending-face '((t :inherit shadow :underline t))
  "Face for a valid entity that is not in the local db yet.")
(defface nostrdown-embed-card-face '((t :inherit (font-lock-doc-face fixed-pitch)))
  "Face for the inline transclusion card.")

;;;; Buffer context (file-local safe)

(defvar-local nostrdown-publication nil
  "The containing publication: a `30040:<pubkey>:<d-tag>' coordinate or an naddr.
Nil means the sibling set comes from `nostrdown-siblings' or the directory.")
(put 'nostrdown-publication 'safe-local-variable #'stringp)

(defvar-local nostrdown-coord nil
  "Coordinate of the event this buffer holds (`kind:pubkey:d-tag').
Set on event buffers opened by `nostrdown-follow-at-point' so the engine can
derive the containing publication itself.")
(put 'nostrdown-coord 'safe-local-variable #'stringp)

(defvar-local nostrdown-author nil
  "Hex pubkey preferred for wiki lookups.")
(put 'nostrdown-author 'safe-local-variable #'stringp)

(defvar-local nostrdown-siblings nil
  "Explicit sibling sections as a list of (TITLE . D-TAG).
When nil and the buffer visits a file, the other files in its directory
are used (basename = d-tag).")
(put 'nostrdown-siblings 'safe-local-variable #'listp)

;;;; Token regexp (display-only; the engine owns the grammar)

(defconst nostrdown-token-regexp
  (concat "{{"
          "\\(\\(?:ref\\|wiki\\|embed\\|quote\\|slot\\|cite\\):\\|@\\)" ; 1 prefix
          "\\([^|}\n]*\\)"                                            ; 2 target
          "\\(?:\\(|\\)\\([^}]*\\)\\)?"                                ; 3 bar, 4 display
          "}}")
  "Locates `{{ }}' tokens for font-lock and point-motion commands.")

(defconst nostrdown-wikilink-regexp
  "\\[\\[\\([^][|\n]+\\)\\(?:\\]\\[\\([^]\n]+\\)\\||\\([^]\n]+\\)\\)?\\]\\]"
  "Locates `[[topic]]' / `[[topic][display]]' / `[[topic|alias]]' wikilinks.")

(defvar nostrdown-font-lock-keywords
  `((,nostrdown-token-regexp
     (0 'nostrdown-delimiter-face prepend)
     (1 'nostrdown-prefix-face prepend)
     (2 'nostrdown-target-face prepend)
     (4 'nostrdown-display-face prepend t))))

;;;; Engine client

(defvar nostrdown--engine-alive nil
  "Cons of (TIME . ALIVE-P) from the last `/health' probe.")

(defun nostrdown--url (path)
  (concat (string-remove-suffix "/" nostrdown-engine-url) path))

(defun nostrdown--parse-response ()
  "Parse the JSON body of the `url-retrieve' buffer.  Return (STATUS . DATA)."
  (goto-char (point-min))
  (let ((status (and (re-search-forward "^HTTP/[0-9.]+ \\([0-9]+\\)" nil t)
                     (string-to-number (match-string 1))))
        (body-start (cond ((and (boundp 'url-http-end-of-headers)
                                (markerp url-http-end-of-headers))
                           (1+ (marker-position url-http-end-of-headers)))
                          ((progn (goto-char (point-min))
                                  (re-search-forward "^\r?$" nil t))
                           (1+ (point))))))
    (cons status
          (when (and body-start (<= body-start (point-max)))
            (condition-case nil
                (json-parse-string
                 (decode-coding-string
                  (buffer-substring-no-properties body-start (point-max)) 'utf-8)
                 :object-type 'alist :array-type 'list
                 :null-object nil :false-object nil)
              (error nil))))))

(defun nostrdown--request (method path payload callback &optional sync)
  "Send METHOD PATH with JSON PAYLOAD; call CALLBACK with (STATUS DATA).
STATUS is nil when the engine did not answer.  With SYNC, block."
  (let ((url-request-method method)
        (url-request-extra-headers
         (and payload '(("Content-Type" . "application/json"))))
        (url-request-data
         (and payload (encode-coding-string (json-serialize payload) 'utf-8)))
        (url-show-status nil)
        (url (nostrdown--url path)))
    (if sync
        (let ((buf (condition-case nil
                       (url-retrieve-synchronously url t t nostrdown-request-timeout)
                     (error nil))))
          (if (not (buffer-live-p buf))
              (funcall callback nil nil)
            (unwind-protect
                (let ((r (with-current-buffer buf (nostrdown--parse-response))))
                  (funcall callback (car r) (cdr r)))
              (kill-buffer buf))))
      (condition-case nil
          (url-retrieve
           url
           (lambda (status)
             (let ((buf (current-buffer)))
               (unwind-protect
                   (if (plist-get status :error)
                       (funcall callback nil nil)
                     (let ((r (nostrdown--parse-response)))
                       (funcall callback (car r) (cdr r))))
                 (when (buffer-live-p buf) (kill-buffer buf)))))
           nil t t)
        (error (funcall callback nil nil))))))

(defun nostrdown--engine-available-p ()
  "Non-nil when the engine answered `/health' within the last 30 seconds."
  (let ((now (float-time)))
    (when (or (null nostrdown--engine-alive)
              (> (- now (car nostrdown--engine-alive)) 30))
      (let ((nostrdown-request-timeout 2) (alive nil))
        (nostrdown--request "GET" "/health" nil
                            (lambda (status _) (setq alive (eq status 200)))
                            t)
        (setq nostrdown--engine-alive (cons now alive))))
    (cdr nostrdown--engine-alive)))

(defun nostrdown--backend ()
  (pcase nostrdown-backend
    ('tendrl 'tendrl)
    ('nak 'nak)
    (_ (if (nostrdown--engine-available-p) 'tendrl 'nak))))

;;;; Sibling context

(defvar nostrdown--siblings-cache nil
  "Alist of DIRECTORY -> (TIME . SIBLINGS).")

(defun nostrdown--file-title (file)
  "First title-like line of FILE.
An Org `#+TITLE:', a Markdown / AsciiDoc heading, or front-matter `title:'."
  (with-temp-buffer
    (condition-case nil
        (insert-file-contents file nil 0 4000)
      (error nil))
    (goto-char (point-min))
    (let ((case-fold-search t))
      (when (re-search-forward
             (concat "^\\(?:#\\+title:\\|title:\\|=\\|#\\)[ \t]+\"?\\([^\"\n]+?\\)\"?[ \t]*$")
             nil t)
        (string-trim (match-string 1))))))

(defun nostrdown--directory-siblings (dir)
  "Sibling sections derived from the files in DIR: (TITLE . BASENAME)."
  (let* ((dir (file-name-as-directory (expand-file-name dir)))
         (hit (assoc dir nostrdown--siblings-cache)))
    (if (and hit (< (- (float-time) (cadr hit)) 10))
        (cddr hit)
      (let ((sibs
             (delq nil
                   (mapcar
                    (lambda (f)
                      (when (and (file-regular-p f)
                                 (member (downcase (or (file-name-extension f) ""))
                                         nostrdown-sibling-extensions))
                        (cons (or (nostrdown--file-title f) (file-name-base f))
                              (file-name-base f))))
                    (directory-files dir t "\\`[^.#]" t)))))
        (setq nostrdown--siblings-cache
              (cons (cons dir (cons (float-time) sibs))
                    (assoc-delete-all dir nostrdown--siblings-cache)))
        sibs))))

(defun nostrdown--siblings ()
  (or nostrdown-siblings
      (and buffer-file-name
           (nostrdown--directory-siblings (file-name-directory buffer-file-name)))))

(defun nostrdown--keyword (name)
  "Value of a `#+NAME:' / `NAME:' line near the top of the buffer, or nil."
  (save-excursion
    (save-restriction
      (widen)
      (goto-char (point-min))
      (let ((case-fold-search t))
        (when (re-search-forward
               (format "^[ \t]*\\(?:#\\+\\)?%s:[ \t]*\\(.+?\\)[ \t]*$" (regexp-quote name))
               (min (point-max) 4000) t)
          (match-string-no-properties 1))))))

(defvar-local nostrdown--publication-coord nil
  "Cached coordinate form of `nostrdown-publication'.")

(defun nostrdown--publication ()
  "The containing publication as a `30040:pk:d' coordinate, or nil."
  (let ((raw (or nostrdown-publication (nostrdown--keyword "nostrdown_publication"))))
    (when raw
      (setq raw (string-remove-prefix "nostr:" (string-trim raw)))
      (cond
       ((string-match-p "\\`[0-9]+:[0-9a-f]\\{64\\}:" raw) raw)
       ((and nostrdown--publication-coord
             (equal (car nostrdown--publication-coord) raw))
        (cdr nostrdown--publication-coord))
       ((string-prefix-p "naddr1" raw)
        (let (coord)
          (nostrdown--request
           "POST" "/api/v1/decode" `((input . ,raw))
           (lambda (status data)
             (when (and (eq status 200) (equal (alist-get 'kind data) "naddr"))
               (setq coord (format "%s:%s:%s" (alist-get 'kind_int data)
                                   (alist-get 'pubkey data) (alist-get 'd_tag data)))))
           t)
          (when coord (setq nostrdown--publication-coord (cons raw coord)))
          coord))))))

(defun nostrdown--author ()
  (or nostrdown-author (nostrdown--keyword "nostrdown_author")))

;;;; Resolution state

(defvar-local nostrdown--refs nil "The refs painted by the last resolve.")
(defvar-local nostrdown--timer nil)
(defvar-local nostrdown--status nil "Short status for the lighter.")
(defvar-local nostrdown--revealed nil "The overlay whose raw text is currently shown.")

(defun nostrdown--lighter ()
  (format " ND[%s%s]" (or nostrdown--status "·")
          (if (eq (nostrdown--backend-cached) 'nak) " nak" "")))

(defun nostrdown--backend-cached ()
  "`nostrdown--backend' without re-probing the engine (for the mode line)."
  (pcase nostrdown-backend
    ('tendrl 'tendrl) ('nak 'nak)
    (_ (if (cdr nostrdown--engine-alive) 'tendrl 'nak))))

;;;; UTF-16 spans -> buffer positions

(defun nostrdown--utf16-index (content)
  "Vector of UTF-16 offsets at which astral (two-unit) characters start in CONTENT."
  (let ((u 0) (acc nil))
    (dotimes (i (length content))
      (if (> (aref content i) #xFFFF)
          (progn (push u acc) (setq u (+ u 2)))
        (setq u (1+ u))))
    (vconcat (nreverse acc))))

(defun nostrdown--utf16->pos (u index)
  "Buffer position (widened, 1-based) of UTF-16 offset U given INDEX."
  (let ((n 0) (len (length index)))
    (while (and (< n len) (< (aref index n) u))
      (setq n (1+ n)))
    (+ 1 (- u n))))

;;;; Overlays

(defvar nostrdown-overlay-map
  (let ((m (make-sparse-keymap)))
    (define-key m (kbd "RET") #'nostrdown-follow-at-point)
    (define-key m (kbd "C-c C-o") #'nostrdown-follow-at-point)
    (define-key m [mouse-1] #'nostrdown-follow-mouse)
    (define-key m [mouse-2] #'nostrdown-follow-mouse)
    m)
  "Keymap active on a resolved reference.")

(defun nostrdown--overlay-at (pos)
  (seq-find (lambda (o) (overlay-get o 'nostrdown)) (overlays-at pos)))

(defun nostrdown--overlays ()
  (save-restriction
    (widen)
    (seq-filter (lambda (o) (overlay-get o 'nostrdown))
                (overlays-in (point-min) (point-max)))))

(defun nostrdown--clear-overlays ()
  (setq nostrdown--revealed nil)
  (save-restriction
    (widen)
    (remove-overlays (point-min) (point-max) 'nostrdown t)
    (remove-overlays (point-min) (point-max) 'nostrdown-child t)))

(defun nostrdown--state-face (ref)
  (cond ((alist-get 'pending ref) 'nostrdown-pending-face)
        ((alist-get 'found ref) 'nostrdown-resolved-face)
        (t 'nostrdown-unresolved-face)))

(defun nostrdown--pretty-label (ref)
  "The label shown for REF when `nostrdown-prettify' is `label'."
  (let ((kind (alist-get 'kind ref))
        (label (or (alist-get 'label ref) (alist-get 'target ref) "")))
    (propertize
     (pcase kind
       ("mention" (concat "@" (string-remove-prefix "@" label)))
       ("embed" (concat "⊞ " label))
       ("slot" (concat "⧉ " label))
       ("quote" (format "“%s”%s" label
                        (if-let* ((title (alist-get 'title ref)))
                            (concat " — " title) "")))
       (_ label))
     'face (nostrdown--state-face ref))))

(defun nostrdown--hide-markup (ov)
  "Add invisible child overlays over the `{{prefix:' … `}}' chrome of OV."
  (let ((beg (overlay-start ov)) (end (overlay-end ov)) (children nil))
    (save-excursion
      (goto-char beg)
      (when (and (looking-at nostrdown-token-regexp) (= (match-end 0) end))
        (let ((mk (lambda (a b)
                    (let ((c (make-overlay a b nil t nil)))
                      (overlay-put c 'nostrdown-child t)
                      (overlay-put c 'invisible 'nostrdown)
                      (overlay-put c 'evaporate t)
                      (push c children)))))
          (if (match-beginning 4)
              ;; {{ref:target|display}} -> show only the display text.
              (progn (funcall mk beg (match-beginning 4))
                     (funcall mk (match-end 4) end))
            ;; {{ref:target}} -> show only the target.
            (funcall mk beg (match-beginning 2))
            (funcall mk (match-end 2) end)))))
    (overlay-put ov 'nostrdown-children children)))

(defun nostrdown--embed-card (ref)
  "An `after-string' card for an embed / quote / slot REF."
  (let* ((title (or (alist-get 'title ref) (alist-get 'label ref) ""))
         (author (or (alist-get 'author ref)
                     (when-let* ((pk (alist-get 'author_pubkey ref)))
                       (concat (substring pk 0 8) "…"))))
         (content (alist-get 'content ref))
         (lines (and content (split-string (string-trim content) "\n")))
         (shown (and lines (seq-take lines nostrdown-embed-max-lines)))
         (body (cond
                (shown (mapconcat (lambda (l) (concat "  ┃ " l)) shown "\n"))
                ((alist-get 'pending ref)
                 "  ┃ (not in the local db — nostrdown-fetch-at-point pulls it)")
                ((alist-get 'found ref) "  ┃ (no content)")
                (t "  ┃ (unresolved)"))))
    (propertize
     (concat "\n  ┏ " title (if author (concat " · " author) "")
             (if-let* ((k (alist-get 'event_kind ref))) (format " · kind %s" k) "")
             "\n" body
             (if (and lines (> (length lines) (length shown))) "\n  ┃ …" "")
             "\n  ┗")
     'face 'nostrdown-embed-card-face)))

(defun nostrdown--describe (ref)
  "A multi-line description of REF.
Used for `help-echo' and `nostrdown-describe-at-point'."
  (let ((kind (alist-get 'kind ref)))
    (string-join
     (delq nil
           (list
            (format "%s → %s" kind
                    (cond ((alist-get 'pending ref) "pending (not local yet)")
                          ((alist-get 'found ref) "resolved")
                          (t "unresolved")))
            (when-let* ((title (alist-get 'title ref))) (concat "title: " title))
            (when-let* ((s (alist-get 'summary ref))) (concat "summary: " s))
            (when-let* ((a (alist-get 'author ref))) (concat "author: " a))
            (when-let* ((pk (alist-get 'author_pubkey ref))) (concat "pubkey: " pk))
            (when-let* ((k (alist-get 'event_kind ref))) (format "kind: %s" k))
            (when-let* ((c (alist-get 'coord ref))) (concat "coord: " c))
            (when-let* ((n (alist-get 'naddr ref))) (concat "naddr: " n))
            (when-let* ((e (alist-get 'event_id ref))) (concat "event: " e))
            (when-let* ((ts (alist-get 'created_at ref)))
              (concat "created: " (format-time-string "%Y-%m-%d %H:%M" ts)))
            (concat "target: " (or (alist-get 'target ref) ""))
            (if (alist-get 'found ref) "RET / mouse-1: follow"
              (pcase kind
                ("wiki" "RET: look the topic up on the relays")
                ("ref" "no sibling section has this handle")
                (_ nil)))))
     "\n")))

(defun nostrdown--help-echo (_window object pos)
  (let* ((ov (if (overlayp object) object
               (with-current-buffer (if (bufferp object) object (current-buffer))
                 (nostrdown--overlay-at pos))))
         (ref (and ov (overlay-get ov 'nostrdown-ref))))
    (and ref (nostrdown--describe ref))))

(defun nostrdown--decorate (ov ref)
  "Paint overlay OV according to resolved REF."
  (overlay-put ov 'nostrdown t)
  (overlay-put ov 'nostrdown-ref ref)
  (overlay-put ov 'evaporate t)
  (overlay-put ov 'face (nostrdown--state-face ref))
  (overlay-put ov 'mouse-face 'highlight)
  (overlay-put ov 'keymap nostrdown-overlay-map)
  (overlay-put ov 'help-echo #'nostrdown--help-echo)
  (dolist (c (overlay-get ov 'nostrdown-children)) (delete-overlay c))
  (overlay-put ov 'nostrdown-children nil)
  (overlay-put ov 'display nil)
  (overlay-put ov 'nostrdown-display nil)
  (pcase nostrdown-prettify
    ('label (let ((l (nostrdown--pretty-label ref)))
              (overlay-put ov 'nostrdown-display l)
              (overlay-put ov 'display l)))
    ('markup (nostrdown--hide-markup ov)))
  (overlay-put ov 'after-string
               (and nostrdown-inline-embeds
                    (member (alist-get 'kind ref) '("embed" "quote" "slot"))
                    (nostrdown--embed-card ref))))

(defun nostrdown--apply-spans (spans)
  "Paint SPANS, a list of (BEG END REF), replacing the previous overlays."
  (nostrdown--clear-overlays)
  (let ((found 0) (refs nil))
    (save-restriction
      (widen)
      (dolist (s spans)
        (let ((ov (make-overlay (nth 0 s) (nth 1 s) nil t nil)))
          (nostrdown--decorate ov (nth 2 s))
          (push (nth 2 s) refs)
          (when (alist-get 'found (nth 2 s)) (setq found (1+ found))))))
    (setq nostrdown--refs (nreverse refs)
          nostrdown--status (format "%d/%d" found (length spans)))
    (force-mode-line-update)))

(defun nostrdown--redecorate ()
  "Re-paint the existing overlays (after a display preference changed)."
  (dolist (ov (nostrdown--overlays))
    (nostrdown--decorate ov (overlay-get ov 'nostrdown-ref))))

;;;; Resolution: tendrl backend

(defun nostrdown--context-item (content)
  (let ((pub (nostrdown--publication))
        (author (nostrdown--author))
        (sibs (nostrdown--siblings)))
    (append
     `((key . "buffer") (content . ,content))
     (and pub `((publication . ,pub)))
     (and nostrdown-coord `((coord . ,nostrdown-coord)))
     (and author `((author . ,author)))
     `((siblings . ,(vconcat
                     (mapcar (lambda (s) `((title . ,(car s)) (d_tag . ,(cdr s))))
                             sibs)))))))

(defun nostrdown--resolve-tendrl (policy &optional sync)
  (let* ((buf (current-buffer))
         (tick (buffer-chars-modified-tick))
         (content (save-restriction (widen)
                                    (buffer-substring-no-properties (point-min) (point-max))))
         (payload `((items . [,(nostrdown--context-item content)])
                    (policy . ,policy))))
    (setq nostrdown--status "…")
    (force-mode-line-update)
    (nostrdown--request
     "POST" "/api/v1/nostrdown/resolve" payload
     (lambda (status data)
       (when (buffer-live-p buf)
         (with-current-buffer buf
           (cond
            ((not (eq status 200))
             (setq nostrdown--status "!")
             (setq nostrdown--engine-alive (cons (float-time) nil))
             (force-mode-line-update)
             (message "nostrdown: engine at %s did not answer (%s)"
                      nostrdown-engine-url (or status "no connection")))
            ((/= tick (buffer-chars-modified-tick))
             ;; Edited while the request was in flight — the timer will re-run.
             (nostrdown--schedule))
            (t
             (let* ((refs (alist-get 'buffer (alist-get 'refs data)))
                    (index (nostrdown--utf16-index content)))
               (nostrdown--apply-spans
                (mapcar (lambda (ref)
                          (list (nostrdown--utf16->pos (alist-get 'start ref) index)
                                (nostrdown--utf16->pos (alist-get 'end ref) index)
                                ref))
                        refs))))))))
     sync)))

;;;; Resolution: nak fallback

(defun nostrdown--entity-p (target)
  (string-match-p "\\`\\(?:nostr:\\)?\\(?:naddr\\|nevent\\|note\\|nprofile\\|npub\\)1[a-z0-9]+\\'"
                  target))

(defun nostrdown--markup-target-p (target)
  "Non-nil when a `[[ ]]' target belongs to the host markup (link, path, image)."
  (or (string-match-p "\\`[a-zA-Z][a-zA-Z0-9+.-]*:" target)
      (string-match-p "://\\|\\`[./~]\\|\\`#\\|\\`\\*" target)
      (string-match-p "\\.\\(?:png\\|jpe?g\\|gif\\|svg\\|webp\\|pdf\\|org\\|md\\|html?\\)\\'" target)))

(defun nostrdown--slug (s)
  "A NIP-54-style slug of S (fallback only; the engine normalises for real)."
  (let ((x (downcase (string-trim s))))
    (setq x (replace-regexp-in-string "[ \t_]+" "-" x))
    (setq x (replace-regexp-in-string "[^a-z0-9-]" "" x))
    (string-trim x "-+" "-+")))

(defun nostrdown--scan-tokens ()
  "Scan the widened buffer; return a list of (BEG END KIND TARGET DISPLAY)."
  (let (out)
    (save-excursion
      (save-restriction
        (widen)
        (goto-char (point-min))
        (while (re-search-forward nostrdown-token-regexp nil t)
          (let* ((prefix (match-string-no-properties 1))
                 (kind (if (equal prefix "@") "mention" (string-remove-suffix ":" prefix))))
            (push (list (match-beginning 0) (match-end 0) kind
                        (string-trim (match-string-no-properties 2))
                        (and (match-beginning 4) (string-trim (match-string-no-properties 4))))
                  out)))
        (goto-char (point-min))
        (while (re-search-forward nostrdown-wikilink-regexp nil t)
          (let ((target (string-trim (match-string-no-properties 1))))
            (unless (nostrdown--markup-target-p target)
              (push (list (match-beginning 0) (match-end 0) "wiki" target
                          (or (match-string-no-properties 2) (match-string-no-properties 3)))
                    out))))))
    (nreverse out)))

(defun nostrdown--event-title (event)
  "Title of a nak-fetched EVENT alist: `title' tag, or kind-0 name."
  (let ((tags (alist-get 'tags event)))
    (or (seq-some (lambda (tag) (and (equal (car tag) "title") (cadr tag))) tags)
        (when (eql (alist-get 'kind event) 0)
          (condition-case nil
              (let ((m (json-parse-string (alist-get 'content event)
                                          :object-type 'alist :null-object nil)))
                (or (alist-get 'display_name m) (alist-get 'name m)))
            (error nil))))))

(defun nostrdown--nak-fetch (entity callback)
  "Run `nak fetch ENTITY' asynchronously.
CALLBACK receives the first event as an alist, or nil."
  (let ((buf (generate-new-buffer " *nostrdown-nak*")))
    (condition-case nil
        ;; nak blocks on an open stdin: hand it EOF right away.
        (process-send-eof
         (make-process
         :name "nostrdown-nak" :buffer buf :noquery t
         :command (list nostrdown-nak-program "fetch" (string-remove-prefix "nostr:" entity))
         :sentinel
         (lambda (proc _event)
           (unless (process-live-p proc)
             (let ((event
                    (with-current-buffer buf
                      (goto-char (point-min))
                      (when (re-search-forward "^{" nil t)
                        (condition-case nil
                            (json-parse-string
                             (buffer-substring-no-properties
                              (match-beginning 0) (line-end-position))
                             :object-type 'alist :array-type 'list
                             :null-object nil :false-object nil)
                          (error nil))))))
               (kill-buffer buf)
               (funcall callback event))))))
      (error (kill-buffer buf) (funcall callback nil)))))

(defun nostrdown--resolve-nak ()
  "Resolve with the nak fallback: entities via `nak fetch', refs via siblings."
  (let* ((buf (current-buffer))
         (sibs (nostrdown--siblings))
         (spans
          (mapcar
           (lambda (tok)
             (pcase-let ((`(,beg ,end ,kind ,target ,display) tok))
               (let* ((entity (nostrdown--entity-p target))
                      (sib (and (not entity) (member kind '("ref" "embed" "wiki"))
                                (seq-find (lambda (s)
                                            (or (equal (cdr s) target)
                                                (equal (nostrdown--slug (car s))
                                                       (nostrdown--slug target))
                                                (equal (nostrdown--slug (cdr s))
                                                       (nostrdown--slug target))))
                                          sibs)))
                      (ref `((kind . ,kind)
                             (target . ,(if entity (string-remove-prefix "nostr:" target)
                                          (nostrdown--slug target)))
                             (label . ,(or display (and sib (car sib))
                                           (if (and entity (> (length target) 20))
                                               (concat (substring target 0 10) "…"
                                                       (substring target -4))
                                             target)))
                             (found . ,(and (or sib entity) t))
                             (pending . ,(and entity t))
                             ,@(and sib `((title . ,(car sib))
                                          (coord . ,(format "30041::%s" (cdr sib))))))))
                 (list beg end ref))))
           (nostrdown--scan-tokens))))
    (nostrdown--apply-spans spans)
    ;; Fill entity labels in as nak answers.
    (dolist (ov (nostrdown--overlays))
      (let ((ref (overlay-get ov 'nostrdown-ref)))
        (when (alist-get 'pending ref)
          (nostrdown--nak-fetch
           (alist-get 'target ref)
           (lambda (event)
             (when (and (buffer-live-p buf) (overlay-buffer ov))
               (with-current-buffer buf
                 (let ((ref (overlay-get ov 'nostrdown-ref)))
                   (if (null event)
                       (setf (alist-get 'found ref) nil)
                     (setf (alist-get 'title ref) (nostrdown--event-title event))
                     (unless (alist-get 'label-explicit ref)
                       (setf (alist-get 'label ref)
                             (or (nostrdown--event-title event) (alist-get 'label ref))))
                     (setf (alist-get 'content ref) (alist-get 'content event))
                     (setf (alist-get 'event_kind ref) (alist-get 'kind event))
                     (setf (alist-get 'event_id ref) (alist-get 'id event))
                     (setf (alist-get 'author_pubkey ref) (alist-get 'pubkey event))
                     (setf (alist-get 'created_at ref) (alist-get 'created_at event)))
                   (setf (alist-get 'pending ref) nil)
                   (nostrdown--decorate ov ref)))))))))))

;;;; Scheduling

(defun nostrdown--schedule ()
  (when nostrdown--timer (cancel-timer nostrdown--timer))
  (let ((buf (current-buffer)))
    (setq nostrdown--timer
          (run-with-idle-timer
           nostrdown-idle-delay nil
           (lambda ()
             (when (buffer-live-p buf)
               (with-current-buffer buf
                 (setq nostrdown--timer nil)
                 (when nostrdown-mode (nostrdown-resolve-buffer)))))))))

(defun nostrdown--after-change (_beg _end _len)
  (nostrdown--schedule))

(defun nostrdown--post-command ()
  "Reveal the raw token under point; re-hide the one point left."
  (let ((ov (nostrdown--overlay-at (point))))
    (unless (eq ov nostrdown--revealed)
      (when (and nostrdown--revealed (overlay-buffer nostrdown--revealed))
        (let ((old nostrdown--revealed))
          (overlay-put old 'display (overlay-get old 'nostrdown-display))
          (dolist (c (overlay-get old 'nostrdown-children))
            (overlay-put c 'invisible 'nostrdown))))
      (setq nostrdown--revealed ov)
      (when ov
        (overlay-put ov 'display nil)
        (dolist (c (overlay-get ov 'nostrdown-children))
          (overlay-put c 'invisible nil))))))

;;;; Commands

(defun nostrdown-resolve-buffer (&optional policy)
  "Resolve every reference in the buffer against the engine (or nak).
POLICY overrides `nostrdown-idle-policy'."
  (interactive)
  (pcase (nostrdown--backend)
    ('tendrl (nostrdown--resolve-tendrl (or policy nostrdown-idle-policy)))
    ('nak (nostrdown--resolve-nak))))

(defun nostrdown-resolve-buffer-sync (&optional policy)
  "Like `nostrdown-resolve-buffer' but block until the engine answers."
  (interactive)
  (pcase (nostrdown--backend)
    ('tendrl (nostrdown--resolve-tendrl (or policy nostrdown-idle-policy) t))
    ('nak (nostrdown--resolve-nak))))

(defun nostrdown-resolve-online ()
  "Resolve with `nostrdown-online-policy': unresolved wiki topics and entities
are looked up on the relays.  In the engine's Confirm mode this pass silently
stays local; approve the fetch in the tendrl UI instead."
  (interactive)
  (nostrdown-resolve-buffer nostrdown-online-policy))

(defun nostrdown-describe-at-point ()
  "Show what the reference at point resolved to."
  (interactive)
  (let ((ov (nostrdown--overlay-at (point))))
    (unless ov (user-error "No nostrdown reference at point"))
    (message "%s" (nostrdown--describe (overlay-get ov 'nostrdown-ref)))))

(defun nostrdown-next-reference (&optional n)
  "Move point to the start of the next (with N < 0, previous) reference."
  (interactive "p")
  (let* ((ovs (sort (nostrdown--overlays)
                    (lambda (a b) (< (overlay-start a) (overlay-start b)))))
         (target (if (>= (or n 1) 0)
                     (seq-find (lambda (o) (> (overlay-start o) (point))) ovs)
                   (seq-find (lambda (o) (< (overlay-start o) (point))) (reverse ovs)))))
    (if target (goto-char (overlay-start target))
      (user-error "No further nostrdown reference"))))

(defun nostrdown-previous-reference ()
  "Move point to the start of the previous reference."
  (interactive)
  (nostrdown-next-reference -1))

(defun nostrdown-cycle-prettify ()
  "Cycle `nostrdown-prettify' through off, hidden markup, and resolved label."
  (interactive)
  (setq nostrdown-prettify (pcase nostrdown-prettify ('nil 'markup) ('markup 'label) (_ nil)))
  (setq nostrdown--revealed nil)
  (nostrdown--redecorate)
  (message "nostrdown prettify: %s" (or nostrdown-prettify "off")))

(defun nostrdown-toggle-inline-embeds ()
  "Toggle the inline transclusion cards under embed / quote / slot tokens."
  (interactive)
  (setq nostrdown-inline-embeds (not nostrdown-inline-embeds))
  (nostrdown--redecorate)
  (message "nostrdown inline embeds: %s" (if nostrdown-inline-embeds "on" "off")))

(defun nostrdown-insert-reference (prefix target display)
  "Insert a `{{PREFIX:TARGET|DISPLAY}}' token at point.
Sibling handles complete for ref / embed targets."
  (interactive
   (let* ((prefix (completing-read "Reference: " '("ref" "wiki" "embed" "quote" "slot" "@") nil t))
          (sibs (nostrdown--siblings))
          (target (completing-read
                   (format "{{%s%s " prefix (if (equal prefix "@") "" ":"))
                   (if (member prefix '("ref" "embed"))
                       (append (mapcar #'cdr sibs) (mapcar #'car sibs))
                     nil)))
          (display (read-string (if (equal prefix "quote") "Excerpt: " "Display text (optional): "))))
     (list prefix target display)))
  (insert (format "{{%s%s%s%s}}" prefix (if (equal prefix "@") "" ":") target
                  (if (string-empty-p display) "" (concat "|" display)))))

(defun nostrdown-follow-mouse (event)
  "Follow the reference clicked in EVENT."
  (interactive "e")
  (let ((pos (posn-point (event-start event))))
    (with-current-buffer (window-buffer (posn-window (event-start event)))
      (nostrdown-follow-at-point pos))))

(defun nostrdown-follow-at-point (&optional pos)
  "Follow the reference at POS (point): visit the sibling file, open the
event in a buffer, show the profile, or offer a relay lookup."
  (interactive)
  (let* ((ov (nostrdown--overlay-at (or pos (point))))
         (ref (and ov (overlay-get ov 'nostrdown-ref))))
    (unless ref (user-error "No nostrdown reference at point"))
    (nostrdown--follow ref)))

(defun nostrdown--follow (ref)
  (let ((kind (alist-get 'kind ref))
        (coord (alist-get 'coord ref))
        (eid (alist-get 'event_id ref))
        (found (alist-get 'found ref)))
    (cond
     ((and coord (string-match "\\`[0-9]+::\\(.+\\)\\'" coord))
      (nostrdown--visit-sibling (match-string 1 coord) ref))
     ((alist-get 'pending ref)
      (if (eq (nostrdown--backend) 'nak)
          (nostrdown--nak-show (alist-get 'target ref))
        (nostrdown-fetch-at-point)))
     ((not found)
      (pcase kind
        ("wiki" (when (y-on-p-compat
                       (format "`%s' is not local.  Look it up on the relays? "
                               (alist-get 'label ref)))
                  (nostrdown-resolve-online)))
        ("ref" (message "nostrdown: no sibling section answers to `%s'"
                        (alist-get 'target ref)))
        (_ (message "nostrdown: unresolved %s `%s'" kind (alist-get 'target ref)))))
     ((equal kind "mention") (nostrdown--show-profile ref))
     ((and coord (string-match "\\`\\([0-9]+\\):\\([0-9a-f]+\\):\\(.*\\)\\'" coord))
      (nostrdown--show-addressable (match-string 1 coord) (match-string 2 coord)
                                   (match-string 3 coord)))
     (eid (nostrdown--show-event eid))
     ((and (eq (nostrdown--backend) 'nak) (nostrdown--entity-p (alist-get 'target ref)))
      (nostrdown--nak-show (alist-get 'target ref)))
     (t (message "nostrdown: nothing to open for %s" kind)))))

(defun y-on-p-compat (prompt)
  "`y-or-n-p', but answer yes non-interactively (batch tests)."
  (if noninteractive t (y-or-n-p prompt)))

(defun nostrdown--visit-sibling (d-tag ref)
  "Visit the sibling file whose basename is D-TAG, else describe REF."
  (let* ((dir (and buffer-file-name (file-name-directory buffer-file-name)))
         (file (and dir
                    (seq-some (lambda (ext)
                                (let ((f (expand-file-name (concat d-tag "." ext) dir)))
                                  (and (file-exists-p f) f)))
                              nostrdown-sibling-extensions))))
    (if file (find-file file)
      (message "nostrdown: sibling `%s' — %s" d-tag (or (alist-get 'title ref) "no file here")))))

(defun nostrdown--format-mode (event)
  "Major mode for EVENT's content, from its format / m tags."
  (let* ((tags (alist-get 'tags event))
         (fmt (downcase
               (or (seq-some (lambda (tg) (and (member (car tg) '("format" "m" "M")) (cadr tg)))
                             tags)
                   ""))))
    (cond ((string-match-p "org" fmt) 'org-mode)
          ((and (string-match-p "markdown\\|\\bmd\\b" fmt) (fboundp 'markdown-mode)) 'markdown-mode)
          ((and (string-match-p "asciidoc\\|adoc" fmt) (fboundp 'adoc-mode)) 'adoc-mode)
          ((and (derived-mode-p 'org-mode 'markdown-mode 'adoc-mode)) major-mode)
          (t 'text-mode))))

(defun nostrdown--show-event-buffer (event)
  "Display EVENT (an alist) in a read-only buffer with nostrdown-mode on."
  (let* ((tags (alist-get 'tags event))
         (title (or (seq-some (lambda (tg) (and (equal (car tg) "title") (cadr tg))) tags)
                    (format "kind %s" (alist-get 'kind event))))
         (d (seq-some (lambda (tg) (and (equal (car tg) "d") (cadr tg))) tags))
         (mode (nostrdown--format-mode event))
         (buf (get-buffer-create (format "*nostrdown: %s*" title))))
    (with-current-buffer buf
      (let ((inhibit-read-only t))
        (erase-buffer)
        (insert (or (alist-get 'content event) ""))
        (goto-char (point-min)))
      (funcall mode)
      (setq header-line-format
            (format " %s · kind %s · %s… · %s" title (alist-get 'kind event)
                    (substring (or (alist-get 'pubkey event) "????????") 0 8)
                    (format-time-string "%Y-%m-%d" (or (alist-get 'created_at event) 0))))
      (setq nostrdown-coord (and d (format "%s:%s:%s" (alist-get 'kind event)
                                           (alist-get 'pubkey event) d)))
      (setq nostrdown-author (alist-get 'pubkey event))
      (setq nostrdown-siblings nil)
      (view-mode 1)
      (nostrdown-mode 1)
      (nostrdown-resolve-buffer))
    (pop-to-buffer buf)
    buf))

(defun nostrdown--show-addressable (kind pubkey d-tag)
  (nostrdown--request
   "GET" (format "/api/v1/addressable/%s/%s/%s?policy=local_first"
                 kind pubkey (url-hexify-string d-tag))
   nil
   (lambda (status data)
     (if (and (eq status 200) (alist-get 'event data))
         (nostrdown--show-event-buffer (alist-get 'event data))
       (message "nostrdown: could not load %s:%s:%s (%s)" kind pubkey d-tag status)))))

(defun nostrdown--show-event (id)
  (nostrdown--request
   "GET" (format "/api/v1/events/%s?policy=local_first" id) nil
   (lambda (status data)
     (if (and (eq status 200) (or (alist-get 'event data) (alist-get 'id data)))
         (nostrdown--show-event-buffer (or (alist-get 'event data) data))
       (message "nostrdown: could not load event %s (%s)" id status)))))

(defun nostrdown--show-profile (ref)
  (let ((pk (or (alist-get 'author_pubkey ref) (alist-get 'target ref))))
    (nostrdown--request
     "GET" (format "/api/v1/profile/%s?policy=local_first" pk) nil
     (lambda (status data)
       (if (not (eq status 200))
           (message "nostrdown: profile %s not loaded (%s)" pk status)
         (with-current-buffer (get-buffer-create
                               (format "*nostrdown: @%s*" (or (alist-get 'name data) pk)))
           (let ((inhibit-read-only t))
             (erase-buffer)
             (dolist (k '(display_name name nip05 about picture pubkey))
               (when-let* ((v (alist-get k data)))
                 (insert (format "%-13s %s\n" (symbol-name k) v))))
             (goto-char (point-min)))
           (view-mode 1)
           (pop-to-buffer (current-buffer))))))))

(defun nostrdown--nak-show (entity)
  (nostrdown--nak-fetch
   entity
   (lambda (event)
     (if event (nostrdown--show-event-buffer event)
       (message "nostrdown: nak found nothing for %s" entity)))))

(defun nostrdown-fetch-at-point ()
  "Pull the pending entity at point from the engine's search relays.
In the engine's Confirm mode the request waits for approval in the tendrl UI."
  (interactive)
  (let* ((ov (nostrdown--overlay-at (point)))
         (ref (and ov (overlay-get ov 'nostrdown-ref)))
         (buf (current-buffer)))
    (unless ref (user-error "No nostrdown reference at point"))
    (unless (alist-get 'pending ref) (user-error "This reference is not pending"))
    (message "nostrdown: fetching %s… (Confirm mode waits for approval in tendrl)"
             (alist-get 'label ref))
    (nostrdown--request
     "POST" "/api/v1/nostrdown/fetch-entity"
     `((entity . ,(alist-get 'target ref)) (want_content . t))
     (lambda (status data)
       (when (and (buffer-live-p buf) (overlay-buffer ov))
         (with-current-buffer buf
           (if (not (eq status 200))
               (message "nostrdown: fetch failed (%s)" status)
             (let ((new (alist-get 'ref data)))
               ;; The engine echoes spans relative to the entity alone; keep ours.
               (setf (alist-get 'start new) (alist-get 'start ref)
                     (alist-get 'end new) (alist-get 'end ref))
               (nostrdown--decorate ov new)
               (message "nostrdown: %s" (if (alist-get 'pending new)
                                            "relays had nothing" "fetched"))))))))))

;;;; Completion

(defun nostrdown-completion-at-point ()
  "Complete `{{' prefixes and sibling handles after `{{ref:' / `{{embed:'."
  (let ((bol (line-beginning-position)))
    (cond
     ((looking-back "{{\\(?:ref\\|embed\\|wiki\\):\\([^|}\n]*\\)" bol)
      (let* ((beg (match-beginning 1))
             (sibs (nostrdown--siblings))
             (table (append (mapcar #'cdr sibs) (mapcar #'car sibs))))
        (list beg (point) table
              :exclusive 'no
              :annotation-function
              (lambda (c)
                (when-let* ((s (seq-find (lambda (s) (equal (cdr s) c)) sibs)))
                  (concat "  " (car s)))))))
     ((looking-back "{{\\([a-z@]*\\)" bol)
      (list (match-beginning 1) (point) '("ref:" "wiki:" "embed:" "quote:" "slot:" "@")
            :exclusive 'no)))))

;;;; Mode

(defvar nostrdown-command-map
  (let ((m (make-sparse-keymap)))
    (define-key m (kbd "r") #'nostrdown-resolve-buffer)
    (define-key m (kbd "R") #'nostrdown-resolve-online)
    (define-key m (kbd "f") #'nostrdown-follow-at-point)
    (define-key m (kbd "F") #'nostrdown-fetch-at-point)
    (define-key m (kbd "d") #'nostrdown-describe-at-point)
    (define-key m (kbd "n") #'nostrdown-next-reference)
    (define-key m (kbd "p") #'nostrdown-previous-reference)
    (define-key m (kbd "i") #'nostrdown-insert-reference)
    (define-key m (kbd "e") #'nostrdown-toggle-inline-embeds)
    (define-key m (kbd "l") #'nostrdown-cycle-prettify)
    m)
  "Commands under `nostrdown-keymap-prefix'.")

(defvar nostrdown-mode-map
  (let ((m (make-sparse-keymap)))
    (define-key m (kbd nostrdown-keymap-prefix) nostrdown-command-map)
    m)
  "Keymap for `nostrdown-mode'.")

(defun nostrdown--enable ()
  (setq-local nostrdown-prettify nostrdown-prettify)
  (setq-local nostrdown-inline-embeds nostrdown-inline-embeds)
  (add-to-invisibility-spec 'nostrdown)
  (font-lock-add-keywords nil nostrdown-font-lock-keywords 'append)
  (font-lock-flush)
  (add-hook 'after-change-functions #'nostrdown--after-change nil t)
  (add-hook 'post-command-hook #'nostrdown--post-command nil t)
  (add-hook 'completion-at-point-functions #'nostrdown-completion-at-point nil t)
  (nostrdown--schedule))

(defun nostrdown--disable ()
  (when nostrdown--timer (cancel-timer nostrdown--timer) (setq nostrdown--timer nil))
  (remove-hook 'after-change-functions #'nostrdown--after-change t)
  (remove-hook 'post-command-hook #'nostrdown--post-command t)
  (remove-hook 'completion-at-point-functions #'nostrdown-completion-at-point t)
  (font-lock-remove-keywords nil nostrdown-font-lock-keywords)
  (font-lock-flush)
  (remove-from-invisibility-spec 'nostrdown)
  (nostrdown--clear-overlays)
  (setq nostrdown--refs nil nostrdown--status nil))

;;;###autoload
(defun nostrdown (&optional arg)
  "Overlay the buffer's nostrdown references with their resolved events.
Turns `nostrdown-mode' on (if needed) and resolves right away, blocking
briefly for the engine so the overlays are there when the command returns.
With a prefix ARG, turn the layer off instead."
  (interactive "P")
  (if arg
      (progn (nostrdown-mode -1) (message "nostrdown: off"))
    (unless nostrdown-mode (nostrdown-mode 1))
    (when nostrdown--timer (cancel-timer nostrdown--timer) (setq nostrdown--timer nil))
    (nostrdown-resolve-buffer-sync)
    (message "nostrdown: %s references resolved via %s%s"
             (or nostrdown--status "0/0") (nostrdown--backend)
             (if (eq (nostrdown--backend) 'tendrl) (concat " at " nostrdown-engine-url) ""))))

;;;###autoload
(define-minor-mode nostrdown-mode
  "Nostr {{ }} references over the buffer's own markup.
Tokens are fontified at once; a running tendrl engine (or nak) resolves
them and the results are painted as overlays."
  :lighter (:eval (nostrdown--lighter))
  :keymap nostrdown-mode-map
  (if nostrdown-mode (nostrdown--enable) (nostrdown--disable)))

(defun nostrdown--turn-on ()
  (when (and (not (minibufferp))
             (apply #'derived-mode-p nostrdown-modes))
    (nostrdown-mode 1)))

;;;###autoload
(define-globalized-minor-mode global-nostrdown-mode nostrdown-mode nostrdown--turn-on
  :group 'nostrdown)

(provide 'nostrdown-mode)
;;; nostrdown-mode.el ends here
