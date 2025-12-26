from setuptools import setup, find_packages

setup(
    name="solidb",
    version="0.1.0",
    description="Python client for SoliDB",
    author="SoliDB Team",
    author_email="team@solisoft.net",
    packages=find_packages(),
    install_requires=[
        "msgpack>=1.0.5",
    ],
    python_requires=">=3.7",
)
