//! ml — machine learning written in olang itself.
//!
//! No native ML kernels are involved: logistic regression, k-means,
//! and the evaluation metrics below are ordinary olang functions whose
//! hot loops the engine promotes and compiles like any other code.
//! Features are columns — plain lists of Floats — so the training
//! loops read contiguous data the way the numeric kernels do.

/// Deterministic split: every `k`-th index goes to the test set. On
/// shuffled-enough data (crimes arrive in no meaningful order after
/// sampling) this is an honest split without needing a seeded shuffle.
share fn train_test_indices(n, k) = {
    let mut train = []
    let mut test = []
    for i in 0..n {
        if i % k == 0 => { test = test +[i] }
        else => { train = train + [i] }
    }
    #{ "train": train, "test": test }
}

fn sigmoid(z) =
    if z > 35.0 => 1.0
    else if z < -35.0 => 0.0
    else => 1.0 / (1.0 + math.exp(0.0 - z))

/// Logistic regression by full-batch gradient descent.
/// `cols` is the feature matrix as a list of columns (lists of Float),
/// `labels` a list of 0.0/1.0, `epochs` and `rate` the schedule.
/// Returns #{ "weights", "bias", "loss_curve" } — the loss curve is
/// recorded every 10 epochs so convergence is inspectable.
/// The optional `on_epoch(done, total, latest_loss)` callback runs
/// after every epoch (latest_loss is the most recent entry of the
/// loss curve, 0.0 before the first is computed) — outside the hot
/// loops, so the native tier is unaffected.
share fn logistic_train(cols, labels, epochs, rate,
                        on_epoch = (done, total, loss) => ()) = {
    let n_features = len(cols)
    let n = len(labels)
    let mut weights = map(0..n_features, (j) => 0.0)
    let mut bias = 0.0
    let mut losses = []
    let mut last_loss = 0.0

    let mut epoch = 0
    while epoch < epochs {
        // Forward pass: predictions for every row.
        let mut preds = []
        for i in 0..n {
            let mut z = bias
            for j in 0..n_features {
                z = z + weights[j] * cols[j][i]
            }
            preds = preds + [sigmoid(z)]
        }
        // Gradients.
        let mut grad_b = 0.0
        let mut grads = map(0..n_features, (j) => 0.0)
        for i in 0..n {
            let err = preds[i] - labels[i]
            grad_b = grad_b + err
            for j in 0..n_features {
                grads = col.set(grads, j, grads[j] + err * cols[j][i])
            }
        }
        let scale = rate / to_float(n)
        weights = map(0..n_features, (j) => weights[j] - scale * grads[j])
        bias = bias - scale * grad_b

        if epoch % 10 == 0 => {
            let mut loss = 0.0
            for i in 0..n {
                let p = math.max(0.000001, math.min(0.999999, preds[i]))
                loss = loss - (labels[i] * math.ln(p) + (1.0 - labels[i]) * math.ln(1.0 - p))
            }
            last_loss = math.round(loss / to_float(n) * 10000.0) / 10000.0
            losses = losses + [last_loss]
        }
        epoch = epoch + 1
        on_epoch(epoch, epochs, last_loss)
    }
    #{ "weights": weights, "bias": bias, "loss_curve": losses }
}

/// Predicted probabilities for a feature matrix under a trained model.
share fn logistic_predict(model, cols, n) = {
    let weights = map_get(model, "weights")
    let bias = map_get(model, "bias")
    let n_features = len(cols)
    let mut preds = []
    for i in 0..n {
        let mut z = bias
        for j in 0..n_features {
            z = z + weights[j] * cols[j][i]
        }
        preds = preds + [sigmoid(z)]
    }
    preds
}

/// Confusion counts and derived metrics at a 0.5 threshold.
share fn classification_report(preds, labels) = {
    let mut tp = 0
    let mut tn = 0
    let mut fp = 0
    let mut fn_ = 0
    for i in 0..len(labels) {
        let hit = preds[i] >= 0.5
        let pos = labels[i] >= 0.5
        if hit && pos => { tp = tp + 1 }
        else if hit && !pos => { fp = fp + 1 }
        else if !hit && pos => { fn_ = fn_ + 1 }
        else => { tn = tn + 1 }
    }
    let total = to_float(tp + tn + fp + fn_)
    let accuracy = to_float(tp + tn) / total
    let precision = if tp + fp == 0 => 0.0 else => to_float(tp) / to_float(tp + fp)
    let recall = if tp + fn_ == 0 => 0.0 else => to_float(tp) / to_float(tp + fn_)
    let f1 = if precision + recall == 0.0 => 0.0
        else => 2.0 * precision * recall / (precision + recall)
    #{ "tp": tp, "tn": tn, "fp": fp, "fn": fn_,
       "accuracy": accuracy, "precision": precision, "recall": recall, "f1": f1 }
}

/// AUC by rank statistic (the probability a random positive scores
/// above a random negative) — threshold-free, so it complements the
/// 0.5-threshold report above.
share fn auc(preds, labels) = {
    let n = len(labels)
    let mut pairs = []
    for i in 0..n { pairs = pairs + [[preds[i], labels[i]]] }
    let ranked = col.sort_by(pairs, (p) => p[0])
    let mut rank_sum = 0.0
    let mut positives = 0
    for i in 0..n {
        if ranked[i][1] >= 0.5 => {
            rank_sum = rank_sum + to_float(i + 1)
            positives = positives + 1
        }
    }
    let negatives = n - positives
    if positives == 0 || negatives == 0 => return 0.5
    (rank_sum - to_float(positives * (positives + 1)) / 2.0)
        / (to_float(positives) * to_float(negatives))
}

/// Lloyd's k-means on 2-D points. Centroids seed from evenly spaced
/// points (deterministic); iterates assignment/update `iters` times.
/// Returns #{ "cx", "cy", "assignment", "sizes" }.
/// The optional `on_iter(done, total)` callback runs after each
/// Lloyd iteration.
share fn kmeans2(xs, ys, k, iters, on_iter = (done, total) => ()) = {
    let n = len(xs)
    let stride = n / k
    let mut cx = map(0..k, (c) => xs[c * stride])
    let mut cy = map(0..k, (c) => ys[c * stride])
    let mut assignment = map(0..n, (i) => 0)

    let mut it = 0
    while it < iters {
        // Assign every point to its nearest centroid.
        for i in 0..n {
            let mut best = 0
            let mut best_d = 100000000.0
            for c in 0..k {
                let dx = xs[i] - cx[c]
                let dy = ys[i] - cy[c]
                let d = dx * dx + dy * dy
                if d < best_d => { best_d = d; best = c }
            }
            assignment = col.set(assignment, i, best)
        }
        // Move each centroid to the mean of its members.
        let mut sum_x = map(0..k, (c) => 0.0)
        let mut sum_y = map(0..k, (c) => 0.0)
        let mut count = map(0..k, (c) => 0)
        for i in 0..n {
            let c = assignment[i]
            sum_x = col.set(sum_x, c, sum_x[c] + xs[i])
            sum_y = col.set(sum_y, c, sum_y[c] + ys[i])
            count = col.set(count, c, count[c] + 1)
        }
        for c in 0..k {
            if count[c] > 0 => {
                cx = col.set(cx, c, sum_x[c] / to_float(count[c]))
                cy = col.set(cy, c, sum_y[c] / to_float(count[c]))
            }
        }
        it = it + 1
        on_iter(it, iters)
    }
    let mut sizes = map(0..k, (c) => 0)
    for i in 0..n { sizes = col.set(sizes, assignment[i], sizes[assignment[i]] + 1) }
    let mut inertia = 0.0
    for i in 0..n {
        let c = assignment[i]
        let dx = xs[i] - cx[c]
        let dy = ys[i] - cy[c]
        inertia = inertia + dx * dx + dy * dy
    }
    #{ "cx": cx, "cy": cy, "assignment": assignment, "sizes": sizes, "inertia": inertia }
}

/// Min-max normalize a list of Floats into [0, 1].
share fn normalize(values) = {
    let lo = fold(values, values[0], (a, v) => if v < a => v else => a)
    let hi = fold(values, values[0], (a, v) => if v > a => v else => a)
    let range = hi - lo
    if range == 0.0 => map(values, (v) => 0.0)
    else => map(values, (v) => (v - lo) / range)
}

test "logistic regression separates a separable problem" {
    // One feature: negative for class 0, positive for class 1.
    let x = [-2.0, -1.5, -1.0, -0.5, 0.5, 1.0, 1.5, 2.0]
    let y = [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]
    let m = logistic_train([x], y, 200, 1.0)
    let preds = logistic_predict(m, [x], 8)
    let rep = classification_report(preds, y)
    assert_eq(map_get(rep, "accuracy"), 1.0)
    assert_eq(auc(preds, y), 1.0)
    // The loss curve must fall.
    let losses = map_get(m, "loss_curve")
    assert(losses[len(losses) - 1] < losses[0])
}

test "kmeans separates two obvious blobs" {
    let xs = [0.0, 0.1, 0.2, 0.1, 10.0, 10.1, 10.2, 9.9]
    let ys = [0.0, 0.1, 0.0, 0.2, 5.0, 5.1, 4.9, 5.0]
    let km = kmeans2(xs, ys, 2, 10)
    let sizes = map_get(km, "sizes")
    assert_eq(sort(sizes), [4, 4])
    let a = map_get(km, "assignment")
    assert_eq(a[0], a[1])
    assert_eq(a[4], a[5])
    assert(a[0] != a[4])
}

test "classification metrics count the confusion square" {
    let rep = classification_report([0.9, 0.9, 0.1, 0.1, 0.9], [1.0, 1.0, 0.0, 1.0, 0.0])
    assert_eq(map_get(rep, "tp"), 2)
    assert_eq(map_get(rep, "tn"), 1)
    assert_eq(map_get(rep, "fp"), 1)
    assert_eq(map_get(rep, "fn"), 1)
}

test "normalize maps to the unit interval" {
    assert_eq(normalize([2.0, 4.0, 6.0]), [0.0, 0.5, 1.0])
}


/// Confusion counts and derived metrics at an arbitrary threshold.
share fn classification_report_at(preds, labels, threshold) = {
    let mut tp = 0
    let mut tn = 0
    let mut fp = 0
    let mut fn_ = 0
    for i in 0..len(labels) {
        let hit = preds[i] >= threshold
        let pos = labels[i] >= 0.5
        if hit && pos => { tp = tp + 1 }
        else if hit && !pos => { fp = fp + 1 }
        else if !hit && pos => { fn_ = fn_ + 1 }
        else => { tn = tn + 1 }
    }
    let total = to_float(tp + tn + fp + fn_)
    let accuracy = to_float(tp + tn) / total
    let precision = if tp + fp == 0 => 0.0 else => to_float(tp) / to_float(tp + fp)
    let recall = if tp + fn_ == 0 => 0.0 else => to_float(tp) / to_float(tp + fn_)
    let f1 = if precision + recall == 0.0 => 0.0
        else => 2.0 * precision * recall / (precision + recall)
    #{ "tp": tp, "tn": tn, "fp": fp, "fn": fn_, "threshold": threshold,
       "accuracy": accuracy, "precision": precision, "recall": recall, "f1": f1 }
}

/// Sweep the decision threshold and return the F1-optimal operating
/// point plus the (threshold, precision, recall, f1) curve — the
/// honest way to report an imbalanced classifier, where the default
/// 0.5 cut can sit below the base rate and predict nothing positive.
share fn threshold_sweep(preds, labels) = {
    let mut best = classification_report_at(preds, labels, 0.5)
    let mut curve = []
    for t in 1..20 {
        let th = to_float(t) * 0.05
        let r = classification_report_at(preds, labels, th)
        curve = curve + [[th, map_get(r, "precision"), map_get(r, "recall"), map_get(r, "f1")]]
        if map_get(r, "f1") > map_get(best, "f1") => { best = r }
    }
    #{ "best": best, "curve": curve }
}

/// Reliability pairs for a calibration diagram: rows of
/// [mean predicted probability, empirical positive rate, count] over
/// `bins` equal-width probability bins (empty bins are dropped).
share fn calibration(preds, labels, bins) = {
    let n = len(labels)
    let mut cnt = map(0..bins, (b) => 0)
    let mut psum = map(0..bins, (b) => 0.0)
    let mut lsum = map(0..bins, (b) => 0.0)
    for i in 0..n {
        let mut b = to_int(preds[i] * to_float(bins))
        if b >= bins => { b = bins - 1 }
        cnt = col.set(cnt, b, cnt[b] + 1)
        psum = col.set(psum, b, psum[b] + preds[i])
        lsum = col.set(lsum, b, lsum[b] + labels[i])
    }
    let mut rows = []
    for b in 0..bins {
        if cnt[b] > 0 => {
            rows = rows + [[psum[b] / to_float(cnt[b]), lsum[b] / to_float(cnt[b]), to_float(cnt[b])]]
        }
    }
    rows
}

/// A 95% confidence interval for an AUC by the Hanley–McNeil (1982)
/// standard error — analytic, so it costs nothing at any n.
share fn auc_ci(a, n_pos, n_neg) = {
    let q1 = a / (2.0 - a)
    let q2 = 2.0 * a * a / (1.0 + a)
    let np = to_float(n_pos)
    let nn = to_float(n_neg)
    let var = (a * (1.0 - a) + (np - 1.0) * (q1 - a * a) + (nn - 1.0) * (q2 - a * a)) / (np * nn)
    let se = math.sqrt(math.max(0.0, var))
    [math.max(0.0, a - 1.96 * se), math.min(1.0, a + 1.96 * se)]
}

/// The Wilson score interval for a proportion (95%): honest error bars
/// for rates estimated from very different sample sizes.
share fn wilson_ci(k, n) = {
    let z = 1.96
    let nf = to_float(n)
    let p = to_float(k) / nf
    let z2 = z * z
    let denom = 1.0 + z2 / nf
    let center = (p + z2 / (2.0 * nf)) / denom
    let half = z * math.sqrt(p * (1.0 - p) / nf + z2 / (4.0 * nf * nf)) / denom
    [math.max(0.0, center - half), math.min(1.0, center + half)]
}

/// Autocorrelation of a series at lags 1..max_lag (demeaned,
/// normalized by lag-0 variance).
share fn acf(xs, max_lag) = {
    let n = len(xs)
    let mut mean = 0.0
    for i in 0..n { mean = mean + xs[i] }
    mean = mean / to_float(n)
    let mut var = 0.0
    for i in 0..n {
        let d = xs[i] - mean
        var = var + d * d
    }
    let mut out = []
    for lag in 1..(max_lag + 1) {
        let mut s = 0.0
        for i in 0..(n - lag) { s = s + (xs[i] - mean) * (xs[i + lag] - mean) }
        out = out + [s / math.max(0.000001, var)]
    }
    out
}

/// Gaussian naive Bayes: closed-form per-class feature means and
/// variances, priors from the label rate. The complement of gradient
/// descent — one pass over the data, no iterations, strong when the
/// class-conditional marginals carry the signal.
share fn gnb_train(cols, labels) = {
    let nf = len(cols)
    let n = len(labels)
    let mut n1 = 0.0
    for i in 0..n { n1 = n1 + labels[i] }
    let n0 = to_float(n) - n1
    let mut mu0 = map(0..nf, (j) => 0.0)
    let mut mu1 = map(0..nf, (j) => 0.0)
    for j in 0..nf {
        let mut s0 = 0.0
        let mut s1 = 0.0
        for i in 0..n {
            if labels[i] >= 0.5 => { s1 = s1 + cols[j][i] }
            else => { s0 = s0 + cols[j][i] }
        }
        mu0 = col.set(mu0, j, s0 / math.max(1.0, n0))
        mu1 = col.set(mu1, j, s1 / math.max(1.0, n1))
    }
    let mut v0 = map(0..nf, (j) => 0.0)
    let mut v1 = map(0..nf, (j) => 0.0)
    for j in 0..nf {
        let mut s0 = 0.0
        let mut s1 = 0.0
        for i in 0..n {
            if labels[i] >= 0.5 => {
                let d = cols[j][i] - mu1[j]
                s1 = s1 + d * d
            } else => {
                let d = cols[j][i] - mu0[j]
                s0 = s0 + d * d
            }
        }
        v0 = col.set(v0, j, s0 / math.max(1.0, n0) + 0.000001)
        v1 = col.set(v1, j, s1 / math.max(1.0, n1) + 0.000001)
    }
    #{ "mu0": mu0, "mu1": mu1, "v0": v0, "v1": v1,
       "logp1": math.ln(math.max(0.000001, n1 / to_float(n))),
       "logp0": math.ln(math.max(0.000001, n0 / to_float(n))) }
}

/// Posterior positive probabilities under a gnb_train model.
share fn gnb_predict(model, cols, n) = {
    let mu0 = map_get(model, "mu0")
    let mu1 = map_get(model, "mu1")
    let v0 = map_get(model, "v0")
    let v1 = map_get(model, "v1")
    let nf = len(mu0)
    let lc0 = map(0..nf, (j) => 0.0 - 0.5 * math.ln(2.0 * 3.141592653589793 * v0[j]))
    let lc1 = map(0..nf, (j) => 0.0 - 0.5 * math.ln(2.0 * 3.141592653589793 * v1[j]))
    let base0 = map_get(model, "logp0")
    let base1 = map_get(model, "logp1")
    let mut preds = []
    for i in 0..n {
        let mut s0 = base0
        let mut s1 = base1
        for j in 0..nf {
            let x = cols[j][i]
            let d0 = x - mu0[j]
            let d1 = x - mu1[j]
            s0 = s0 + lc0[j] - d0 * d0 / (2.0 * v0[j])
            s1 = s1 + lc1[j] - d1 * d1 / (2.0 * v1[j])
        }
        let m = if s0 > s1 => s0 else => s1
        let e0 = math.exp(s0 - m)
        let e1 = math.exp(s1 - m)
        preds = preds + [e1 / (e0 + e1)]
    }
    preds
}

/// A CART decision tree with histogram splits (gini impurity):
/// features are binned once against their global range, each node
/// scans its rows once per feature, and the tree lives in a flat
/// arena of parallel lists (feature -1 marks a leaf). Deterministic —
/// no sampling — so runs reproduce exactly.
share fn tree_train(cols, labels, max_depth, min_leaf) = {
    let nf = len(cols)
    let n = len(labels)
    let bins = 24
    let mins = map(cols, (c) => fold(c, c[0], (a, v) => if v < a => v else => a))
    let spans = map(0..nf, (j) =>
        math.max(0.000001, fold(cols[j], cols[j][0], (a, v) => if v > a => v else => a) - mins[j]))
    let mut nfeat = [0 - 1]
    let mut nthr = [0.0]
    let mut nleft = [0 - 1]
    let mut nright = [0 - 1]
    let mut npred = [0.0]
    let mut q_node = [0]
    let mut q_rows = [map(0..n, (i) => i)]
    let mut q_depth = [0]
    let mut qi = 0
    while qi < len(q_node) {
        let node = q_node[qi]
        let rows = q_rows[qi]
        let depth = q_depth[qi]
        qi = qi + 1
        let m = len(rows)
        let mut pos = 0.0
        for r in rows { pos = pos + labels[r] }
        let pred = pos / to_float(m)
        npred = col.set(npred, node, pred)
        if depth < max_depth && m >= 2 * min_leaf && pos > 0.5 && pos < to_float(m) - 0.5 => {
            // Parent impurity, and the best histogram cut over features.
            let parent_gini = 2.0 * pred * (1.0 - pred)
            let mut best_gain = 0.0000001
            let mut best_f = 0 - 1
            let mut best_thr = 0.0
            for f in 0..nf {
                let mut cnt = map(0..bins, (b) => 0.0)
                let mut cpos = map(0..bins, (b) => 0.0)
                for r in rows {
                    let mut b = to_int((cols[f][r] - mins[f]) / spans[f] * to_float(bins - 1))
                    if b >= bins => { b = bins - 1 }
                    cnt = col.set(cnt, b, cnt[b] + 1.0)
                    cpos = col.set(cpos, b, cpos[b] + labels[r])
                }
                let mut lc = 0.0
                let mut lp = 0.0
                for b in 0..(bins - 1) {
                    lc = lc + cnt[b]
                    lp = lp + cpos[b]
                    let rc = to_float(m) - lc
                    let rp = pos - lp
                    if lc >= to_float(min_leaf) && rc >= to_float(min_leaf) => {
                        let pl = lp / lc
                        let pr = rp / rc
                        let g = parent_gini
                            - (lc / to_float(m)) * 2.0 * pl * (1.0 - pl)
                            - (rc / to_float(m)) * 2.0 * pr * (1.0 - pr)
                        if g > best_gain => {
                            best_gain = g
                            best_f = f
                            best_thr = mins[f] + (to_float(b) + 1.0) / to_float(bins - 1) * spans[f]
                        }
                    }
                }
            }
            if best_f >= 0 => {
                // Strict < mirrors the binning exactly: a value on the
                // boundary belongs to the upper bin, so it must go
                // right — with <=, a cut at the top edge would send
                // every row left and hand a child an empty row set.
                let mut left = []
                let mut right = []
                for r in rows {
                    if cols[best_f][r] < best_thr => { left = left + [r] }
                    else => { right = right + [r] }
                }
                if len(left) > 0 && len(right) > 0 => {
                let li = len(nfeat)
                let ri = li + 1
                nfeat = col.set(nfeat, node, best_f)
                nthr = col.set(nthr, node, best_thr)
                nleft = col.set(nleft, node, li)
                nright = col.set(nright, node, ri)
                nfeat = nfeat + [0 - 1] + [0 - 1]
                nthr = nthr + [0.0] + [0.0]
                nleft = nleft + [0 - 1] + [0 - 1]
                nright = nright + [0 - 1] + [0 - 1]
                npred = npred + [0.0] + [0.0]
                q_node = q_node + [li] + [ri]
                q_rows = q_rows + [left] + [right]
                q_depth = q_depth + [depth + 1] + [depth + 1]
                }
            }
        }
    }
    #{ "feat": nfeat, "thr": nthr, "left": nleft, "right": nright, "pred": npred }
}

/// Leaf positive rates for each row under a tree_train model.
share fn tree_predict(model, cols, n) = {
    let feat = map_get(model, "feat")
    let thr = map_get(model, "thr")
    let left = map_get(model, "left")
    let right = map_get(model, "right")
    let pred = map_get(model, "pred")
    let mut out = []
    for i in 0..n {
        let mut node = 0
        while feat[node] >= 0 {
            if cols[feat[node]][i] < thr[node] => { node = left[node] }
            else => { node = right[node] }
        }
        out = out + [pred[node]]
    }
    out
}

/// The two leading principal components of a feature matrix
/// (standardized), by power iteration with deflation. Returns the
/// component vectors and the share of total variance each explains.
share fn pca2(cols) = {
    let nf = len(cols)
    let n = len(cols[0])
    let mu = map(cols, (c) => {
        let mut s = 0.0
        for v in c { s = s + v }
        s / to_float(n)
    })
    let sd = map(0..nf, (j) => {
        let mut s = 0.0
        for i in 0..n {
            let d = cols[j][i] - mu[j]
            s = s + d * d
        }
        math.max(0.000001, math.sqrt(s / to_float(n)))
    })
    // Standardized covariance (the correlation matrix), nf x nf.
    let mut cov = map(0..nf, (j) => map(0..nf, (k) => 0.0))
    for j in 0..nf {
        let mut row = map(0..nf, (k) => 0.0)
        for k in j..nf {
            let mut s = 0.0
            for i in 0..n {
                s = s + (cols[j][i] - mu[j]) * (cols[k][i] - mu[k])
            }
            row = col.set(row, k, s / (to_float(n) * sd[j] * sd[k]))
        }
        cov = col.set(cov, j, row)
    }
    // Mirror the upper triangle.
    let mut full = map(0..nf, (j) => map(0..nf, (k) => 0.0))
    for j in 0..nf {
        let mut row = map(0..nf, (k) => 0.0)
        for k in 0..nf {
            let v = if k >= j => cov[j][k] else => cov[k][j]
            row = col.set(row, k, v)
        }
        full = col.set(full, j, row)
    }
    let first = power_iter(full, nf)
    let lam1 = map_get(first, "lam")
    let v1 = map_get(first, "v")
    // Deflate and repeat.
    let mut defl = map(0..nf, (j) => map(0..nf, (k) => 0.0))
    for j in 0..nf {
        let mut row = map(0..nf, (k) => 0.0)
        for k in 0..nf {
            row = col.set(row, k, full[j][k] - lam1 * v1[j] * v1[k])
        }
        defl = col.set(defl, j, row)
    }
    let second = power_iter(defl, nf)
    #{ "v1": v1, "v2": map_get(second, "v"),
       "share1": lam1 / to_float(nf), "share2": map_get(second, "lam") / to_float(nf),
       "mu": mu, "sd": sd }
}

fn power_iter(mat, nf) = {
    let mut v = map(0..nf, (j) => 1.0 / math.sqrt(to_float(nf)))
    let mut lam = 0.0
    for it in 0..80 {
        let mut w = map(0..nf, (j) => 0.0)
        for j in 0..nf {
            let mut s = 0.0
            for k in 0..nf { s = s + mat[j][k] * v[k] }
            w = col.set(w, j, s)
        }
        let mut norm = 0.0
        for j in 0..nf { norm = norm + w[j] * w[j] }
        norm = math.max(0.000001, math.sqrt(norm))
        v = map(w, (x) => x / norm)
        lam = norm
    }
    #{ "v": v, "lam": lam }
}

test "gaussian naive bayes separates a separable problem" {
    let x = [-2.0, -1.5, -1.0, -0.5, 0.5, 1.0, 1.5, 2.0]
    let y = [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]
    let m = gnb_train([x], y)
    let p = gnb_predict(m, [x], 8)
    assert_eq(p[0] < 0.5, true)
    assert_eq(p[7] > 0.5, true)
}

test "the tree finds an axis-aligned split" {
    let x = [0.1, 0.2, 0.3, 0.4, 0.6, 0.7, 0.8, 0.9]
    let y = [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]
    let m = tree_train([x], y, 3, 1)
    let p = tree_predict(m, [x], 8)
    assert_eq(p[0] < 0.5, true)
    assert_eq(p[7] > 0.5, true)
}

test "pca finds the dominant direction of correlated features" {
    let a = map(0..200, (i) => to_float(i) / 100.0)
    let b = map(0..200, (i) => to_float(i) / 100.0 + to_float(i % 7) * 0.01)
    let c = map(0..200, (i) => to_float((i * 13) % 17) / 17.0)
    let m = pca2([a, b, c])
    // Two collinear features: the first component carries well over
    // half the standardized variance.
    assert_eq(map_get(m, "share1") > 0.5, true)
}

test "threshold sweep beats the default cut on an imbalanced problem" {
    let preds = [0.1, 0.2, 0.3, 0.35, 0.4, 0.45, 0.3, 0.2, 0.42, 0.44]
    let y = [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0]
    let sw = threshold_sweep(preds, y)
    assert_eq(map_get(map_get(sw, "best"), "f1") > 0.9, true)
}

test "wilson interval brackets the point estimate" {
    let ci = wilson_ci(30, 100)
    assert_eq(ci[0] < 0.3 && 0.3 < ci[1], true)
}

test "acf of a periodic series peaks at its period" {
    let xs = map(0..120, (i) => to_float(i % 12))
    let a = acf(xs, 14)
    assert_eq(a[11] > a[5], true)
    assert_eq(a[11] > 0.5, true)
}
