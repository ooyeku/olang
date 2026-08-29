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
share fn logistic_train(cols, labels, epochs, rate) = {
    let n_features = len(cols)
    let n = len(labels)
    let mut weights = map(0..n_features, (j) => 0.0)
    let mut bias = 0.0
    let mut losses = []

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
            losses = losses + [math.round(loss / to_float(n) * 10000.0) / 10000.0]
        }
        epoch = epoch + 1
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
share fn kmeans2(xs, ys, k, iters) = {
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
    }
    let mut sizes = map(0..k, (c) => 0)
    for i in 0..n { sizes = col.set(sizes, assignment[i], sizes[assignment[i]] + 1) }
    #{ "cx": cx, "cy": cy, "assignment": assignment, "sizes": sizes }
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
