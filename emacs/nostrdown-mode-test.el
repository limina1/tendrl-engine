;;; nostrdown-mode-test.el --- live checks for nostrdown-mode  -*- lexical-binding: t; -*-

;; Needs a tendrl engine on `nostrdown-engine-url' (default :3030) holding the
;; example kasten (30040:dc4cd086…:example) and `nak' on PATH.  Run:
;;
;;   emacs -Q --batch -L emacs -l nostrdown-mode -l emacs/nostrdown-mode-test.el
;;
;; Exits non-zero on any FAIL.

;;; Code:

(require 'nostrdown-mode)
(require 'org)

(defvar ndt--failed 0)
(defun ndt-ok (label cond)
  (unless cond (setq ndt--failed (1+ ndt--failed)))
  (message "%s %s" (if cond "PASS" "FAIL") label))
(defun ndt-wait (pred secs)
  (let ((t0 (float-time)))
    (while (and (not (funcall pred)) (< (- (float-time) t0) secs))
      (accept-process-output nil 0.1))
    (funcall pred)))
(defun ndt-ov-text (o) (buffer-substring-no-properties (overlay-start o) (overlay-end o)))
(defun ndt-sorted-overlays ()
  (sort (nostrdown--overlays) (lambda (a b) (< (overlay-start a) (overlay-start b)))))

(defconst ndt-dir (make-temp-file "nostrdown-kasten-" t))
(defun ndt-write (name text) (with-temp-file (expand-file-name name ndt-dir) (insert text)))
(ndt-write "the-ascent.org" "#+TITLE: The Ascent\n\nUp we go. Back to {{ref:main|the index}}.\n")
(ndt-write "the-return.md" "# The Return\n\nDown again.\n")
(ndt-write "notes.adoc" "= Notes\n\nSome notes.\n")
(ndt-write "main.org" (concat "#+TITLE: Main\n\nSee {{ref:The Ascent}} and {{ref:the-return|going back}} "
                              "then {{embed:notes}}.\nA wiki [[Aesop]] and an org link "
                              "[[file:notes.adoc][notes]] and [[L'Abécédaire]].\n"))

;;; 1. Directory siblings + engine resolve + markup prettify + follow
(find-file (expand-file-name "main.org" ndt-dir))
(org-mode)
(nostrdown-mode 1)
(nostrdown-resolve-buffer-sync)
(message "status=%s backend=%s" nostrdown--status (nostrdown--backend))
(ndt-ok "engine backend in use" (eq (nostrdown--backend) 'tendrl))
(let ((ovs (ndt-sorted-overlays)))
  (ndt-ok "every overlay spans exactly one token"
          (and ovs (seq-every-p (lambda (o)
                                  (let ((s (ndt-ov-text o)))
                                    (or (and (string-prefix-p "{{" s) (string-suffix-p "}}" s))
                                        (and (string-prefix-p "[[" s) (string-suffix-p "]]" s)))))
                                ovs)))
  (ndt-ok "5 refs: 3 tokens + 2 wikilinks, file: link skipped" (= (length nostrdown--refs) 5))
  (ndt-ok "ref/embed found via directory siblings"
          (seq-every-p (lambda (r) (alist-get 'found r))
                       (seq-filter (lambda (r) (member (alist-get 'kind r) '("ref" "embed")))
                                   nostrdown--refs)))
  (let ((o (car ovs)))
    (ndt-ok "markup prettify hides chrome"
            (seq-every-p (lambda (c) (eq (overlay-get c 'invisible) 'nostrdown))
                         (overlay-get o 'nostrdown-children)))
    (goto-char (1+ (overlay-start o))) (nostrdown--post-command)
    (ndt-ok "point inside reveals raw text"
            (seq-every-p (lambda (c) (null (overlay-get c 'invisible)))
                         (overlay-get o 'nostrdown-children)))
    (goto-char (point-max)) (nostrdown--post-command)
    (ndt-ok "leaving re-hides"
            (seq-every-p (lambda (c) (eq (overlay-get c 'invisible) 'nostrdown))
                         (overlay-get o 'nostrdown-children))))
  (goto-char (point-min))
  (re-search-forward "{{ref:the-return") (goto-char (match-beginning 0))
  (nostrdown-follow-at-point)
  (ndt-ok "follow ref visits the sibling file"
          (equal (file-name-nondirectory buffer-file-name) "the-return.md")))

;;; 2. UTF-16 span mapping, label prettify, completion
(with-temp-buffer
  (setq nostrdown-siblings '(("Feature: nostrdown" . "feat-nostrdown")))
  (insert "😀 x {{ref:feat-nostrdown}} 🦠 [[Aesop]] {{wiki:proof of work|PoW}} [[file:foo.org][x]]")
  (setq-local nostrdown-prettify 'label)
  (nostrdown-mode 1)
  (nostrdown-resolve-buffer-sync)
  (let ((ovs (ndt-sorted-overlays)))
    (ndt-ok "astral-safe spans"
            (equal (mapcar #'ndt-ov-text ovs)
                   '("{{ref:feat-nostrdown}}" "[[Aesop]]" "{{wiki:proof of work|PoW}}")))
    (ndt-ok "label prettify shows the resolved title"
            (equal (substring-no-properties (overlay-get (car ovs) 'display)) "Feature: nostrdown"))
    (ndt-ok "unresolved wiki gets the unresolved face"
            (eq (overlay-get (nth 1 ovs) 'face) 'nostrdown-unresolved-face)))
  (goto-char (point-max)) (insert " {{ref:feat")
  (ndt-ok "capf offers the sibling d-tag"
          (member "feat-nostrdown" (nth 2 (nostrdown-completion-at-point)))))

;;; 3. Embed of a local publication: inline card + follow into an event buffer
(with-temp-buffer
  (insert "See {{embed:30040:dc4cd086cd7ce5b1832adf4fdd1211289880d2c7e295bcb0e684c01acee77c06:example}} here.")
  (setq-local nostrdown-inline-embeds t)
  (nostrdown-mode 1)
  (nostrdown-resolve-buffer-sync)
  (let* ((o (car (nostrdown--overlays))) (r (overlay-get o 'nostrdown-ref)))
    (ndt-ok "coordinate embed resolves" (alist-get 'found r))
    (ndt-ok "inline card rendered" (string-match-p "┏" (or (overlay-get o 'after-string) "")))
    (goto-char (overlay-start o))
    (nostrdown-follow-at-point)
    (ndt-ok "follow opens the event buffer"
            (ndt-wait (lambda () (get-buffer "*tendrl: Index — example kasten*")) 5))
    (when-let* ((b (get-buffer "*tendrl: Index — example kasten*")))
      (with-current-buffer b
        (ndt-ok "event buffer carries its coordinate" (stringp nostrdown-coord))))))

;;; 4. nak fallback
(with-temp-buffer
  (setq nostrdown-backend 'nak)
  (setq nostrdown-siblings '(("The Ascent" . "sec-1")))
  (insert "{{ref:The Ascent}} {{embed:naddr1qqrk27rpd4cxcegzyrwye5yxe47wtvvr9t05lhgjzy5f3qxjcl3ft09su6zvqxkwua7qvqcyqqq82kq0dcjku}} [[Aesop]]")
  (nostrdown-mode 1)
  (nostrdown-resolve-buffer-sync)
  (let ((ovs (ndt-sorted-overlays)))
    (ndt-ok "nak: sibling ref found" (alist-get 'found (overlay-get (car ovs) 'nostrdown-ref)))
    (ndt-ok "nak: entity starts pending" (alist-get 'pending (overlay-get (nth 1 ovs) 'nostrdown-ref)))
    (ndt-ok "nak: wiki stays unresolved" (not (alist-get 'found (overlay-get (nth 2 ovs) 'nostrdown-ref))))
    (ndt-ok "nak: fetch settles"
            (ndt-wait (lambda () (not (alist-get 'pending (overlay-get (nth 1 ovs) 'nostrdown-ref)))) 30))
    (message "  nak entity -> label=%s found=%s"
             (alist-get 'label (overlay-get (nth 1 ovs) 'nostrdown-ref))
             (alist-get 'found (overlay-get (nth 1 ovs) 'nostrdown-ref)))))

(delete-directory ndt-dir t)
(message "%s" (if (zerop ndt--failed) "ALL PASS" (format "%d FAILED" ndt--failed)))
(kill-emacs (if (zerop ndt--failed) 0 1))
;;; nostrdown-mode-test.el ends here
