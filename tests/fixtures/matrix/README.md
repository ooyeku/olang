Fixtures for tests/matrix_formula_test.rs: a seeded dataset (data.csv)
and reference.json, the values NumPy 2.5, SciPy 1.18 and pandas 3.0
compute on it: products, the Gram matrix, solve, inverse, Cholesky,
eigh, distances, correlation and covariance, and least-squares fits of
formula designs built by hand with patsy's coding. `make_data.py`
wrote both (`python make_data.py .`). The tests never run Python.
